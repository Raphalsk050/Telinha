'use strict';

const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');

const {
  Updater, isNewer, parseVersion, pickUpdate, swapIn,
} = require('../src/updater');

const DOWNLOAD = 'https://github.com/Raphalsk050/Telinha/releases/download';

function release(tag, overrides = {}) {
  return {
    tag_name: tag,
    draft: false,
    prerelease: false,
    html_url: `https://github.com/Raphalsk050/Telinha/releases/tag/${tag}`,
    assets: [{
      name: 'Telinha.exe',
      browser_download_url: `${DOWNLOAD}/${tag}/Telinha.exe`,
      size: 5,
      digest: `sha256:${crypto.createHash('sha256').update('nova!').digest('hex')}`,
    }],
    ...overrides,
  };
}

function scratchDir(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'telinha-updater-'));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

function fakeFetch(latest, body) {
  return async (url) => (String(url).includes('api.github.com')
    ? new Response(JSON.stringify(latest), { status: 200 })
    : new Response(body, { status: 200 }));
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
  assert.equal(update.url, `${DOWNLOAD}/v0.2.0/Telinha.exe`);
  assert.equal(update.size, 5);
  assert.match(update.sha256, /^[a-f0-9]{64}$/);
  assert.equal(update.page, 'https://github.com/Raphalsk050/Telinha/releases/tag/v0.2.0');
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

test('swapIn puts the new file in place and keeps the old one aside', (t) => {
  const dir = scratchDir(t);
  const target = path.join(dir, 'Telinha.exe');
  fs.writeFileSync(target, 'antiga');
  fs.writeFileSync(`${target}.download`, 'nova');
  swapIn(target, `${target}.download`);
  assert.equal(fs.readFileSync(target, 'utf8'), 'nova');
  assert.equal(fs.readFileSync(`${target}.old`, 'utf8'), 'antiga');
  assert.equal(fs.existsSync(`${target}.download`), false);
});

test('check downloads a newer release, verifies it and swaps it in', async (t) => {
  const dir = scratchDir(t);
  const target = path.join(dir, 'Telinha.exe');
  fs.writeFileSync(target, 'antiga');
  const updater = new Updater({
    currentVersion: '0.1.0', target, fetchImpl: fakeFetch(release('v0.2.0'), 'nova!'),
  });
  const seen = [];
  updater.on('changed', (snapshot) => seen.push(snapshot.status));
  await updater.check();
  assert.equal(fs.readFileSync(target, 'utf8'), 'nova!');
  assert.equal(updater.snapshot().status, 'ready');
  assert.equal(updater.snapshot().version, '0.2.0');
  assert.equal(seen[0], 'downloading');
  assert.equal(seen.at(-1), 'ready');
});

test('check keeps the current file when the download does not match the release', async (t) => {
  const dir = scratchDir(t);
  const target = path.join(dir, 'Telinha.exe');
  fs.writeFileSync(target, 'antiga');
  const updater = new Updater({
    currentVersion: '0.1.0', target, fetchImpl: fakeFetch(release('v0.2.0'), 'outra'),
  });
  let failure = null;
  updater.on('failed', (error) => {
    failure = error;
  });
  await updater.check();
  assert.equal(fs.readFileSync(target, 'utf8'), 'antiga');
  assert.equal(updater.snapshot().status, 'available');
  assert.match(failure.message, /nao confere/);
});

test('check stays quiet when the latest release is the running version', async (t) => {
  const dir = scratchDir(t);
  const target = path.join(dir, 'Telinha.exe');
  fs.writeFileSync(target, 'antiga');
  const updater = new Updater({
    currentVersion: '0.2.0', target, fetchImpl: fakeFetch(release('v0.2.0'), 'nova!'),
  });
  await updater.check();
  assert.equal(updater.snapshot().status, 'idle');
  assert.equal(fs.readFileSync(target, 'utf8'), 'antiga');
});

test('check does not download again a release whose build kept the old version', async (t) => {
  const dir = scratchDir(t);
  const target = path.join(dir, 'Telinha.exe');
  const appliedFile = path.join(dir, 'update-applied.txt');
  fs.writeFileSync(target, 'antiga');
  const options = {
    currentVersion: '0.1.0', target, appliedFile, fetchImpl: fakeFetch(release('v0.2.0'), 'nova!'),
  };
  await new Updater(options).check();
  assert.equal(fs.readFileSync(target, 'utf8'), 'nova!');
  assert.equal(fs.readFileSync(appliedFile, 'utf8'), '0.2.0');

  fs.writeFileSync(target, 'aberta de novo');
  const reopened = new Updater(options);
  await reopened.check();
  assert.equal(reopened.snapshot().status, 'idle');
  assert.equal(fs.readFileSync(target, 'utf8'), 'aberta de novo');
});
