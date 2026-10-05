'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const { EventEmitter } = require('node:events');
const { Readable } = require('node:stream');
const { pipeline } = require('node:stream/promises');

const REPOSITORY = 'Raphalsk050/Telinha';
const RELEASE_URL = `https://api.github.com/repos/${REPOSITORY}/releases/latest`;
const RELEASE_PAGE_PREFIX = `https://github.com/${REPOSITORY}/releases/`;
const DOWNLOAD_PREFIX = `${RELEASE_PAGE_PREFIX}download/`;
const ASSET_NAME = 'Telinha.exe';
const USER_AGENT = 'Telinha-Updater';
const FIRST_CHECK_MS = 10 * 1000;
const CHECK_INTERVAL_MS = 2 * 60 * 60 * 1000;
const API_TIMEOUT_MS = 15000;
const STALL_TIMEOUT_MS = 60000;
const MAX_BYTES = 1024 * 1024 * 1024;
const VERSION_PATTERN = /^v?(\d+)\.(\d+)\.(\d+)$/;
const DIGEST_PATTERN = /^sha256:([a-f0-9]{64})$/;

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
  const label = version.join('.');
  const assets = Array.isArray(release.assets) ? release.assets : [];
  const asset = assets.find((item) => item && item.name === ASSET_NAME);
  const url = String(asset?.browser_download_url ?? '');
  if (!asset || !url.startsWith(DOWNLOAD_PREFIX) || !Number.isInteger(asset.size)
    || asset.size <= 0 || asset.size > MAX_BYTES) {
    return null;
  }
  const page = String(release.html_url ?? '');
  return {
    version: label,
    url,
    size: asset.size,
    sha256: DIGEST_PATTERN.exec(String(asset.digest ?? ''))?.[1] ?? null,
    page: page.startsWith(RELEASE_PAGE_PREFIX) ? page : `${RELEASE_PAGE_PREFIX}latest`,
  };
}

async function download(update, destination, { fetchImpl, onProgress }) {
  // Abrir o arquivo antes mostra logo se a pasta do app nao deixa escrever.
  fs.closeSync(fs.openSync(destination, 'w'));

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

// O Windows nao deixa escrever por cima de um executavel aberto, mas deixa trocar o nome dele.
// O antigo sai do caminho e o novo assume o lugar, entao atalhos continuam valendo.
function swapIn(target, downloaded) {
  const old = `${target}.old`;
  fs.rmSync(old, { force: true });
  fs.renameSync(target, old);
  try {
    fs.renameSync(downloaded, target);
  } catch (error) {
    fs.renameSync(old, target);
    throw error;
  }
}

class Updater extends EventEmitter {
  constructor({ currentVersion, target, appliedFile = null, fetchImpl = fetch }) {
    super();
    this.currentVersion = currentVersion;
    this.target = target;
    this.appliedFile = appliedFile;
    this.fetchImpl = fetchImpl;
    this.state = { status: 'idle', version: null, progress: 0, page: null };
    this.busy = false;
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

  removeLeftovers() {
    for (const leftover of [`${this.target}.old`, `${this.target}.download`]) {
      try {
        fs.rmSync(leftover, { force: true });
      } catch {
        // o executavel antigo ainda pode estar fechando, a proxima abertura apaga
      }
    }
  }

  // Uma release publicada sem subir a versao do app traria um executavel que continua dizendo a
  // versao antiga. Sem lembrar o que ja foi trocado, ele seria baixado de novo a cada abertura.
  alreadyApplied(version) {
    if (!this.appliedFile) {
      return false;
    }
    try {
      return fs.readFileSync(this.appliedFile, 'utf8').trim() === version;
    } catch {
      return false;
    }
  }

  rememberApplied(version) {
    if (!this.appliedFile) {
      return;
    }
    try {
      fs.writeFileSync(this.appliedFile, version);
    } catch {
      // sem a anotacao, o pior caso e baixar a mesma versao mais uma vez
    }
  }

  set(changes) {
    this.state = { ...this.state, ...changes };
    this.emit('changed', this.snapshot());
  }

  async latest() {
    const response = await this.fetchImpl(RELEASE_URL, {
      signal: AbortSignal.timeout(API_TIMEOUT_MS),
      headers: { 'user-agent': USER_AGENT, accept: 'application/vnd.github+json' },
    });
    if (response.status === 404) {
      return null;
    }
    if (!response.ok) {
      throw new Error(`o GitHub respondeu ${response.status}`);
    }
    return pickUpdate(await response.json(), this.currentVersion);
  }

  async check() {
    if (this.busy || this.state.status === 'ready') {
      return;
    }
    this.busy = true;
    let update = null;
    try {
      update = await this.latest();
      if (update && this.alreadyApplied(update.version)) {
        update = null;
      }
      if (!update) {
        return;
      }
      const partial = `${this.target}.download`;
      let shown = -1;
      this.set({ status: 'downloading', version: update.version, progress: 0, page: update.page });
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
      swapIn(this.target, partial);
      this.rememberApplied(update.version);
      this.set({ status: 'ready', progress: 1 });
    } catch (error) {
      // Com a versao nova conhecida mas sem conseguir trocar o arquivo, o app avisa e a pessoa
      // baixa pela pagina da release.
      this.set(update
        ? { status: 'available', progress: 0 }
        : { status: 'idle', version: null, progress: 0, page: null });
      this.emit('failed', error);
    } finally {
      this.busy = false;
    }
  }
}

module.exports = {
  Updater, isNewer, parseVersion, pickUpdate, swapIn,
};
