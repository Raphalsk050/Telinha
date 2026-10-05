'use strict';

const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { test } = require('node:test');

const { DEV_EXE } = require('../src/telinha-process');
const {
  Updater, applyStaged, isNewer, parseVersion, pickUpdate,
} = require('../src/updater');

const REPOSITORY = 'https://github.com/Raphalsk050/Telinha';

function release(tag, overrides = {}) {
  return {
    tag_name: tag,
    draft: false,
    prerelease: false,
    assets: [{
      name: 'Telinha.exe',
      browser_download_url: `${REPOSITORY}/releases/download/${tag}/Telinha.exe`,
      size: 5,
      digest: `sha256:${crypto.createHash('sha256').update('nova!').digest('hex')}`,
    }],
    ...overrides,
  };
}

function scratchDir(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'telinha-updater-'));
  t.after(() => fs.rmSync(dir, {
    recursive: true, force: true, maxRetries: 20, retryDelay: 250,
  }));
  return dir;
}

// Responde como o GitHub: a pagina da ultima release redireciona para a tag, a API descreve o
// arquivo e o resto e o download. calls conta quantas vezes cada um foi pedido.
function fakeGitHub(latest, body) {
  const calls = { page: 0, api: 0, download: 0 };
  const fetchImpl = async (url) => {
    const address = String(url);
    if (address === `${REPOSITORY}/releases/latest`) {
      calls.page += 1;
      return new Response(null, {
        status: 302, headers: { location: `${REPOSITORY}/releases/tag/${latest.tag_name}` },
      });
    }
    if (address.includes('api.github.com')) {
      calls.api += 1;
      return new Response(JSON.stringify(latest), { status: 200 });
    }
    calls.download += 1;
    return new Response(body, { status: 200 });
  };
  return { calls, fetchImpl };
}

function newUpdater(t, { current = '0.1.0', latest = 'v0.2.0', body = 'nova!', apply } = {}) {
  const dir = scratchDir(t);
  const target = path.join(dir, 'Telinha.exe');
  const stageDir = path.join(dir, 'updates');
  fs.writeFileSync(target, 'antiga');
  fs.mkdirSync(stageDir);
  const github = fakeGitHub(release(latest), body);
  const updater = new Updater({
    currentVersion: current, target, stageDir, helper: 'telinha.exe', fetchImpl: github.fetchImpl, apply,
  });
  return {
    updater, target, stageDir, calls: github.calls, staged: path.join(stageDir, 'Telinha-0.2.0.exe'),
  };
}

test('parseVersion accepts plain and v-prefixed versions only', () => {
  assert.deepEqual(parseVersion('v1.20.3'), [1, 20, 3]);
  assert.deepEqual(parseVersion('0.1.0'), [0, 1, 0]);
  assert.equal(parseVersion('v1.2'), null);
  assert.equal(parseVersion('v1.2.3-beta'), null);
  assert.equal(parseVersion(undefined), null);
});

test('isNewer compares numerically, field by field', () => {
  assert.equal(isNewer([0, 10, 0], [0, 9, 9]), true);
  assert.equal(isNewer([1, 0, 0], [0, 99, 99]), true);
  assert.equal(isNewer([0, 1, 0], [0, 1, 0]), false);
  assert.equal(isNewer([0, 1, 0], [0, 1, 1]), false);
});

test('pickUpdate returns the portable asset of a newer release', () => {
  const update = pickUpdate(release('v0.2.0'), '0.1.0');
  assert.equal(update.version, '0.2.0');
  assert.equal(update.url, `${REPOSITORY}/releases/download/v0.2.0/Telinha.exe`);
  assert.equal(update.size, 5);
  assert.match(update.sha256, /^[a-f0-9]{64}$/);
});

test('pickUpdate ignores releases that are not a clean upgrade', () => {
  assert.equal(pickUpdate(release('v0.1.0'), '0.1.0'), null);
  assert.equal(pickUpdate(release('v0.0.9'), '0.1.0'), null);
  assert.equal(pickUpdate(release('v0.2.0', { prerelease: true }), '0.1.0'), null);
  assert.equal(pickUpdate(release('v0.2.0', { draft: true }), '0.1.0'), null);
  assert.equal(pickUpdate(release('v0.2.0', { assets: [] }), '0.1.0'), null);
  assert.equal(pickUpdate(null, '0.1.0'), null);
});

test('pickUpdate refuses an asset with another name or hosted elsewhere', () => {
  const renamed = release('v0.2.0');
  renamed.assets[0].name = 'Outro.exe';
  assert.equal(pickUpdate(renamed, '0.1.0'), null);

  const foreign = release('v0.2.0');
  foreign.assets[0].browser_download_url = 'https://example.com/Telinha.exe';
  assert.equal(pickUpdate(foreign, '0.1.0'), null);
});

test('check downloads a newer release next to the app data and leaves the app file alone', async (t) => {
  const {
    updater, target, staged, calls,
  } = newUpdater(t);
  const seen = [];
  updater.on('changed', (snapshot) => seen.push(snapshot.status));
  await updater.check();
  assert.equal(fs.readFileSync(staged, 'utf8'), 'nova!');
  assert.equal(fs.readFileSync(target, 'utf8'), 'antiga');
  assert.deepEqual(updater.snapshot(), {
    status: 'ready', version: '0.2.0', progress: 1, current: '0.1.0',
  });
  assert.equal(seen[0], 'downloading');
  assert.deepEqual(calls, { page: 1, api: 1, download: 1 });
});

test('check asks only for the release page when there is nothing newer', async (t) => {
  const { updater, calls } = newUpdater(t, { current: '0.2.0' });
  await updater.check();
  assert.equal(updater.snapshot().status, 'idle');
  assert.deepEqual(calls, { page: 1, api: 0, download: 0 });
});

test('check drops a download that does not match the release and waits before retrying', async (t) => {
  const {
    updater, stageDir, calls,
  } = newUpdater(t, { body: 'outra' });
  let failure = null;
  updater.on('failed', (error) => {
    failure = error;
  });
  await updater.check();
  assert.match(failure.message, /nao confere/);
  assert.equal(updater.snapshot().status, 'idle');
  assert.deepEqual(fs.readdirSync(stageDir).filter((name) => name.endsWith('.exe')), []);

  await updater.check();
  assert.equal(calls.page, 1);
});

test('check reuses a release that was already downloaded', async (t) => {
  const { updater, staged, calls } = newUpdater(t);
  fs.writeFileSync(staged, 'nova!');
  await updater.check();
  assert.equal(updater.snapshot().status, 'ready');
  assert.deepEqual(calls, { page: 1, api: 0, download: 0 });
});

test('check does not fetch again a release that was applied but kept the old version', async (t) => {
  const { updater, stageDir, calls } = newUpdater(t);
  fs.writeFileSync(path.join(stageDir, 'applied.txt'), '0.2.0\r\n');
  await updater.check();
  assert.equal(updater.snapshot().status, 'idle');
  assert.deepEqual(calls, { page: 1, api: 0, download: 0 });
});

test('a refused permission blocks the update until someone asks again', async (t) => {
  const applied = [];
  const {
    updater, staged, stageDir,
  } = newUpdater(t, { apply: (request) => applied.push(request) });
  fs.writeFileSync(staged, 'nova!');
  fs.writeFileSync(path.join(stageDir, 'failed.txt'), '0.2.0\r\n');
  await updater.check();
  assert.equal(updater.snapshot().status, 'blocked');
  assert.equal(updater.apply({ waitPid: 1, relaunch: false, elevate: false }), false);
  assert.equal(updater.apply({ waitPid: 1, relaunch: true, elevate: true }), true);
  assert.equal(applied.length, 1);
  assert.equal(fs.existsSync(path.join(stageDir, 'failed.txt')), false);
});

test('apply hands the downloaded file to the helper once', async (t) => {
  const applied = [];
  const {
    updater, target, staged, stageDir,
  } = newUpdater(t, { apply: (request) => applied.push(request) });
  assert.equal(updater.apply({ waitPid: 7, relaunch: true, elevate: true }), false);
  await updater.check();
  assert.equal(updater.apply({ waitPid: 7, relaunch: true, elevate: true }), true);
  assert.equal(updater.apply({ waitPid: 7, relaunch: true, elevate: true }), false);
  assert.deepEqual(applied, [{
    helper: 'telinha.exe',
    source: staged,
    target,
    version: '0.2.0',
    waitPid: 7,
    appliedFile: path.join(stageDir, 'applied.txt'),
    failedFile: path.join(stageDir, 'failed.txt'),
    relaunch: true,
    elevate: true,
  }]);
});

test('start clears interrupted downloads and versions that are no longer newer', (t) => {
  const { updater, stageDir } = newUpdater(t, { current: '0.2.0' });
  for (const name of ['Telinha-0.2.0.exe', 'Telinha-0.1.5.exe', 'Telinha-0.3.0.exe', 'Telinha-0.3.0.exe.part']) {
    fs.writeFileSync(path.join(stageDir, name), 'x');
  }
  updater.removeLeftovers();
  assert.deepEqual(fs.readdirSync(stageDir), ['Telinha-0.3.0.exe']);
});

// Precisa do telinha.exe compilado, que e quem faz a troca.
const helperMissing = process.platform !== 'win32' || !fs.existsSync(DEV_EXE);

test('the helper copies over the app file once the app is gone', { skip: helperMissing }, async (t) => {
  const dir = scratchDir(t);
  const target = path.join(dir, 'Telinha.exe');
  const source = path.join(dir, 'updates', 'Telinha-0.2.0.exe');
  const appliedFile = path.join(dir, 'updates', 'applied.txt');
  fs.mkdirSync(path.dirname(source));
  fs.writeFileSync(target, 'antiga');
  fs.writeFileSync(source, 'nova!');
  const running = spawn(process.execPath, ['-e', 'setTimeout(() => {}, 1500)'], { stdio: 'ignore' });

  applyStaged({
    helper: DEV_EXE,
    source,
    target,
    version: '0.2.0',
    waitPid: running.pid,
    appliedFile,
    failedFile: path.join(dir, 'failed.txt'),
    relaunch: false,
    elevate: false,
  });
  assert.equal(fs.readFileSync(target, 'utf8'), 'antiga');

  for (let waited = 0; waited < 30000 && !fs.existsSync(appliedFile); waited += 250) {
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  assert.equal(fs.readFileSync(target, 'utf8'), 'nova!');
  assert.equal(fs.readFileSync(appliedFile, 'utf8').trim(), '0.2.0');
  assert.equal(fs.existsSync(source), false);
});
