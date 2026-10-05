'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { EventEmitter } = require('node:events');
const { Readable } = require('node:stream');
const { pipeline } = require('node:stream/promises');
const { spawn } = require('node:child_process');

const REPOSITORY = 'Raphalsk050/Telinha';
const RELEASE_API = `https://api.github.com/repos/${REPOSITORY}/releases/latest`;
const RELEASE_PAGE = `https://github.com/${REPOSITORY}/releases/latest`;
const DOWNLOAD_PREFIX = `https://github.com/${REPOSITORY}/releases/download/`;
const ASSET_NAME = 'Telinha.exe';
const USER_AGENT = 'Telinha-Updater';
const FIRST_CHECK_MS = 10 * 1000;
const CHECK_INTERVAL_MS = 60 * 1000;
const RETRY_AFTER_FAILURE_MS = 10 * 60 * 1000;
const REQUEST_TIMEOUT_MS = 15000;
const STALL_TIMEOUT_MS = 60000;
const MAX_BYTES = 1024 * 1024 * 1024;
const VERSION_PATTERN = /^v?(\d+)\.(\d+)\.(\d+)$/;
const TAG_LOCATION_PATTERN = /\/releases\/tag\/(v?\d+\.\d+\.\d+)$/;
const DIGEST_PATTERN = /^sha256:([a-f0-9]{64})$/;
const STAGED_PATTERN = /^Telinha-(\d+\.\d+\.\d+)\.exe$/;
const APPLIED_FILE = 'applied.txt';
const FAILED_FILE = 'failed.txt';
const HELPER_NAME = 'telinha-helper.exe';

function parseVersion(text) {
  const match = VERSION_PATTERN.exec(String(text ?? '').trim());
  return match ? match.slice(1).map(Number) : null;
}

function isNewer(candidate, current) {
  for (let index = 0; index < 3; index += 1) {
    if (candidate[index] !== current[index]) {
      return candidate[index] > current[index];
    }
  }
  return false;
}

function pickUpdate(release, currentVersion) {
  const current = parseVersion(currentVersion);
  const version = parseVersion(release?.tag_name);
  if (!current || !version || release.draft || release.prerelease || !isNewer(version, current)) {
    return null;
  }
  const assets = Array.isArray(release.assets) ? release.assets : [];
  const asset = assets.find((item) => item && item.name === ASSET_NAME);
  const url = String(asset?.browser_download_url ?? '');
  if (!asset || !url.startsWith(DOWNLOAD_PREFIX) || !Number.isInteger(asset.size)
    || asset.size <= 0 || asset.size > MAX_BYTES) {
    return null;
  }
  return {
    version: version.join('.'),
    url,
    size: asset.size,
    sha256: DIGEST_PATTERN.exec(String(asset.digest ?? ''))?.[1] ?? null,
  };
}

async function download(update, destination, { fetchImpl, onProgress }) {
  const controller = new AbortController();
  const stall = setTimeout(() => controller.abort(), STALL_TIMEOUT_MS);
  const hash = crypto.createHash('sha256');
  let received = 0;
  try {
    const response = await fetchImpl(update.url, {
      signal: controller.signal,
      headers: { 'user-agent': USER_AGENT, accept: 'application/octet-stream' },
    });
    if (!response.ok || !response.body) {
      throw new Error(`o download respondeu ${response.status}`);
    }
    const measure = async function* measure(source) {
      for await (const chunk of source) {
        received += chunk.length;
        if (received > update.size) {
          throw new Error('o download veio maior que o anunciado');
        }
        hash.update(chunk);
        stall.refresh();
        onProgress(received / update.size);
        yield chunk;
      }
    };
    await pipeline(Readable.fromWeb(response.body), measure, fs.createWriteStream(destination));
  } finally {
    clearTimeout(stall);
  }
  if (received !== update.size) {
    throw new Error('o download veio incompleto');
  }
  if (update.sha256 && hash.digest('hex') !== update.sha256) {
    throw new Error('o arquivo baixado nao confere com o da release');
  }
}

// Quem troca o executavel e o proprio telinha.exe, no modo apply-update (tools/telinha), depois
// que o app fecha. Ele roda de uma copia em stageDir porque o original mora na pasta temporaria
// que o portatil apaga ao sair, e "detached" tira ele do grupo que o Node mata junto com o app.
function applyStaged({
  helper, source, target, version, waitPid, appliedFile, failedFile, relaunch, elevate,
}) {
  const runner = path.join(path.dirname(source), HELPER_NAME);
  fs.copyFileSync(helper, runner);
  const child = spawn(runner, ['apply-update'], {
    cwd: path.dirname(source),
    detached: true,
    stdio: 'ignore',
    windowsHide: true,
    env: {
      ...process.env,
      TELINHA_UPDATE_SOURCE: source,
      TELINHA_UPDATE_TARGET: target,
      TELINHA_UPDATE_VERSION: version,
      TELINHA_UPDATE_PID: String(waitPid),
      TELINHA_UPDATE_APPLIED: appliedFile,
      TELINHA_UPDATE_FAILED: failedFile,
      TELINHA_UPDATE_RELAUNCH: relaunch ? '1' : '0',
      TELINHA_UPDATE_ELEVATE: elevate ? '1' : '0',
    },
  });
  child.unref();
  return child;
}

// A versao nova e baixada para stageDir e fica "ready". Trocar o executavel so da depois que o
// app fecha, entao quem chama apply() fecha o app em seguida.
class Updater extends EventEmitter {
  constructor({
    currentVersion, target, stageDir, helper, fetchImpl = fetch, apply = applyStaged,
  }) {
    super();
    this.currentVersion = currentVersion;
    this.target = target;
    this.stageDir = stageDir;
    this.helper = helper;
    this.fetchImpl = fetchImpl;
    this.applyImpl = apply;
    this.state = { status: 'idle', version: null, progress: 0 };
    this.busy = false;
    this.applying = false;
    this.retryAt = 0;
    this.timers = [];
  }

  snapshot() {
    return { ...this.state, current: this.currentVersion };
  }

  start() {
    this.removeLeftovers();
    const first = setTimeout(() => this.check(), FIRST_CHECK_MS);
    const repeat = setInterval(() => this.check(), CHECK_INTERVAL_MS);
    first.unref();
    repeat.unref();
    this.timers = [first, repeat];
  }

  stop() {
    for (const timer of this.timers) {
      clearTimeout(timer);
    }
    this.timers = [];
  }

  stagedFile(version) {
    return path.join(this.stageDir, `Telinha-${version}.exe`);
  }

  readMark(name) {
    try {
      return fs.readFileSync(path.join(this.stageDir, name), 'utf8').trim();
    } catch {
      return '';
    }
  }

  // Sobra de download interrompido e versao baixada que ja nao e mais nova que a aberta.
  removeLeftovers() {
    const current = parseVersion(this.currentVersion);
    let names = [];
    try {
      fs.mkdirSync(this.stageDir, { recursive: true });
      names = fs.readdirSync(this.stageDir);
    } catch {
      return;
    }
    for (const name of names) {
      const staged = STAGED_PATTERN.exec(name);
      const stale = staged && current && !isNewer(parseVersion(staged[1]), current);
      if (name.endsWith('.part') || name === HELPER_NAME || stale) {
        try {
          fs.rmSync(path.join(this.stageDir, name), { force: true });
        } catch {
          // fica para a proxima abertura
        }
      }
    }
  }

  set(changes) {
    this.state = { ...this.state, ...changes };
    this.emit('changed', this.snapshot());
  }

  // O redirecionamento da pagina da ultima release diz a tag sem gastar o limite por hora da API,
  // entao da para conferir a cada minuto.
  async latestVersion() {
    const response = await this.fetchImpl(RELEASE_PAGE, {
      method: 'HEAD',
      redirect: 'manual',
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
      headers: { 'user-agent': USER_AGENT },
    });
    const tag = TAG_LOCATION_PATTERN.exec(String(response.headers.get('location') ?? ''));
    return tag ? parseVersion(tag[1]) : null;
  }

  async describe() {
    const response = await this.fetchImpl(RELEASE_API, {
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
      headers: { 'user-agent': USER_AGENT, accept: 'application/vnd.github+json' },
    });
    if (!response.ok) {
      throw new Error(`o GitHub respondeu ${response.status}`);
    }
    return pickUpdate(await response.json(), this.currentVersion);
  }

  async check() {
    if (this.busy || this.applying || this.state.status === 'ready' || Date.now() < this.retryAt) {
      return;
    }
    this.busy = true;
    try {
      const latest = await this.latestVersion();
      const current = parseVersion(this.currentVersion);
      if (!latest || !current || !isNewer(latest, current)) {
        return;
      }
      const version = latest.join('.');
      // Uma release publicada sem subir a versao traria um executavel que continua dizendo a
      // versao antiga, e ele seria baixado e trocado de novo sem parar.
      if (this.readMark(APPLIED_FILE) === version) {
        return;
      }
      if (!fs.existsSync(this.stagedFile(version))) {
        await this.fetchRelease(version);
      }
      this.set({
        status: this.readMark(FAILED_FILE) === version ? 'blocked' : 'ready', version, progress: 1,
      });
    } catch (error) {
      this.retryAt = Date.now() + RETRY_AFTER_FAILURE_MS;
      this.set({ status: 'idle', version: null, progress: 0 });
      this.emit('failed', error);
    } finally {
      this.busy = false;
    }
  }

  async fetchRelease(version) {
    const update = await this.describe();
    if (!update || update.version !== version) {
      throw new Error('a release mudou no meio da conferida');
    }
    const partial = `${this.stagedFile(version)}.part`;
    let shown = -1;
    this.set({ status: 'downloading', version, progress: 0 });
    await download(update, partial, {
      fetchImpl: this.fetchImpl,
      onProgress: (progress) => {
        const percent = Math.floor(progress * 100);
        if (percent !== shown) {
          shown = percent;
          this.set({ progress });
        }
      },
    });
    fs.renameSync(partial, this.stagedFile(version));
  }

  // relaunch abre o app de novo depois da troca. elevate deixa pedir a permissao do Windows quando
  // a pasta do executavel e protegida, e so faz sentido com a pessoa na frente do computador.
  apply({ waitPid, relaunch, elevate }) {
    const { status, version } = this.state;
    // Depois de uma permissao negada, so uma tentativa com a pessoa presente vale a pena.
    if (this.applying || (status !== 'ready' && !(status === 'blocked' && elevate))) {
      return false;
    }
    if (elevate) {
      try {
        fs.rmSync(path.join(this.stageDir, FAILED_FILE), { force: true });
      } catch {
        // a troca nova escreve por cima se falhar de novo
      }
    }
    try {
      this.applyImpl({
        helper: this.helper,
        source: this.stagedFile(version),
        target: this.target,
        version,
        waitPid,
        appliedFile: path.join(this.stageDir, APPLIED_FILE),
        failedFile: path.join(this.stageDir, FAILED_FILE),
        relaunch,
        elevate,
      });
    } catch (error) {
      // sem ajudante o app segue aberto na versao atual
      this.emit('failed', error);
      return false;
    }
    this.applying = true;
    return true;
  }
}

module.exports = {
  Updater, applyStaged, isNewer, parseVersion, pickUpdate,
};
