'use strict';

const { spawn } = require('node:child_process');
const { EventEmitter } = require('node:events');
const fs = require('node:fs');
const path = require('node:path');
const readline = require('node:readline');

const CODE_PREFIX = 'TELINHA1.';
const CODE_MAGIC = 'VExT';
const STOP_GRACE_MS = 4000;
const LIST_TIMEOUT_MS = 15000;
const LOCAL_PORT_MIN = 50000;
const LOCAL_PORT_MAX = 50019;
const DEV_EXE = path.join(
  __dirname, '..', '..', 'build', 'msvc-release', 'tools', 'telinha', 'Release', 'telinha.exe');
const TARGET_KINDS = new Set(['monitor', 'window', 'device']);
const AUDIO_CHOICES = new Set(['system', 'process', 'device', 'none']);
const AUDIO_DEVICE_PATTERN = /^[\w{}.-]{1,127}$/;

function resolveTelinhaExe({ env = process.env, resourcesPath, isPackaged = false } = {}) {
  const candidates = [];
  if (env.TELINHA_EXE) {
    candidates.push(env.TELINHA_EXE);
  }
  if (isPackaged && resourcesPath) {
    candidates.push(path.join(resourcesPath, 'telinha.exe'));
  }
  candidates.push(DEV_EXE);
  return candidates.find((candidate) => fs.existsSync(candidate)) ?? null;
}

function parseEventLine(line) {
  const trimmed = String(line).trim();
  if (!trimmed.startsWith('{')) {
    return null;
  }
  try {
    const value = JSON.parse(trimmed);
    return value && typeof value.event === 'string' ? value : null;
  } catch {
    return null;
  }
}

function normalizeCode(text) {
  return String(text ?? '').replace(/\s+/g, '');
}

function looksLikeCode(text) {
  const code = normalizeCode(text);
  const body = code.startsWith(CODE_PREFIX) ? code.slice(CODE_PREFIX.length) : code;
  return body.length > 16 && body.startsWith(CODE_MAGIC) && /^[A-Za-z0-9_-]+$/.test(body);
}

function withPortRange(args) {
  args.push('--port-min', String(LOCAL_PORT_MIN), '--port-max', String(LOCAL_PORT_MAX));
  return args;
}

function isAudioDeviceId(value) {
  return typeof value === 'string' && AUDIO_DEVICE_PATTERN.test(value);
}

function buildShareArgs({
  targetKind, targetIndex, targetHandle, audio = 'system', audioPid, audioDevice, fps, multi = false,
  excludePid,
} = {}) {
  if (!TARGET_KINDS.has(targetKind)) {
    throw new Error('escolha uma tela, um programa ou uma placa de captura para compartilhar');
  }
  const hasHandle = Number.isSafeInteger(targetHandle) && targetHandle > 0;
  if (!hasHandle && (!Number.isInteger(targetIndex) || targetIndex < 0)) {
    throw new Error('indice de alvo invalido');
  }
  if (!AUDIO_CHOICES.has(audio)) {
    throw new Error('opcao de som invalida');
  }

  const args = hasHandle
    ? ['send', '--json', `--${targetKind}-handle`, String(targetHandle)]
    : ['send', '--json', `--${targetKind}`, String(targetIndex)];
  if (multi) {
    args.push('--multi');
  }
  args.push('--audio', audio);
  if (audio === 'process') {
    if (!Number.isInteger(audioPid) || audioPid <= 0) {
      throw new Error('o som por programa precisa do processo do programa');
    }
    args.push('--audio-pid', String(audioPid));
  }
  if (audio === 'device') {
    if (!isAudioDeviceId(audioDevice)) {
      throw new Error('escolha a entrada de som da placa de captura');
    }
    args.push('--audio-device', audioDevice);
  }
  if (audio === 'system' && Number.isInteger(excludePid) && excludePid > 0) {
    args.push('--audio-exclude-pid', String(excludePid));
  }
  if (fps !== undefined) {
    args.push('--fps', String(fps));
  }
  return withPortRange(args);
}

function buildWatchArgs({ fullscreen = false, audio = true, title = '' } = {}) {
  const args = ['recv', '--json'];
  if (fullscreen) {
    args.push('--fullscreen');
  }
  if (title) {
    args.push('--title', title);
  }
  if (!audio) {
    args.push('--no-audio');
  }
  return withPortRange(args);
}

function listTargets(exe, { timeoutMs = LIST_TIMEOUT_MS } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(exe, ['list', '--json'], {
      windowsHide: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error('o telinha demorou demais para listar as telas'));
    }, timeoutMs);

    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on('close', (code) => {
      clearTimeout(timer);
      const event = stdout
        .split(/\r?\n/)
        .map(parseEventLine)
        .find((value) => value && value.event === 'targets');
      if (!event) {
        reject(new Error(`o telinha nao listou as telas (codigo ${code}) ${stderr.trim()}`));
        return;
      }
      resolve({
        monitors: event.monitors ?? [],
        windows: event.windows ?? [],
        devices: event.devices ?? [],
        audioInputs: event.audio_inputs ?? [],
        monitorsError: event.monitors_error ?? null,
        windowsError: event.windows_error ?? null,
        devicesError: event.devices_error ?? null,
      });
    });
  });
}

class TelinhaSession extends EventEmitter {
  constructor(exe, args) {
    super();
    this.exe = exe;
    this.args = args;
    this.child = null;
    this.stopping = false;
    this.exited = false;
    this.killTimer = null;
  }

  start() {
    this.child = spawn(this.exe, this.args, {
      windowsHide: true,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    this.child.stdin.on('error', () => {});

    readline.createInterface({ input: this.child.stdout }).on('line', (line) => {
      const event = parseEventLine(line);
      if (event) {
        this.emit('event', event);
      } else if (line.trim()) {
        this.emit('log', line);
      }
    });
    readline.createInterface({ input: this.child.stderr }).on('line', (line) => {
      if (line.trim()) {
        this.emit('log', line);
      }
    });

    this.child.on('error', (error) => this.finish({ code: null, error: error.message }));
    this.child.on('exit', (code, signal) => this.finish({ code, signal }));
    return this;
  }

  finish(info) {
    if (this.exited) {
      return;
    }
    this.exited = true;
    clearTimeout(this.killTimer);
    this.emit('exit', { ...info, stopped: this.stopping });
  }

  submitCode(text) {
    if (!this.child || this.exited || !looksLikeCode(text)) {
      return false;
    }
    this.child.stdin.write(`${normalizeCode(text)}\n`);
    return true;
  }

  sendCommand(command) {
    if (!this.child || this.exited || this.stopping) {
      return false;
    }
    this.child.stdin.write(`${JSON.stringify(command)}\n`);
    return true;
  }

  stop() {
    if (!this.child || this.exited || this.stopping) {
      return;
    }
    this.stopping = true;
    try {
      this.child.stdin.write('stop\n');
      this.child.stdin.end();
    } catch {
      // the process may already be closing its pipes
    }
    this.killTimer = setTimeout(() => {
      if (!this.exited) {
        this.child.kill();
      }
    }, STOP_GRACE_MS);
  }
}

module.exports = {
  CODE_PREFIX,
  DEV_EXE,
  LOCAL_PORT_MAX,
  LOCAL_PORT_MIN,
  TelinhaSession,
  buildShareArgs,
  buildWatchArgs,
  isAudioDeviceId,
  listTargets,
  looksLikeCode,
  normalizeCode,
  parseEventLine,
  resolveTelinhaExe,
};
