'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');

const { decodeAvatar } = require('../src/avatars');
const { ProfileStore, cleanStyle, hasStyle } = require('../src/profile');

function scratchFile(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'telinha-profile-'));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return path.join(dir, 'profile.json');
}

function jpeg(text) {
  const data = Buffer.concat([Buffer.from([0xff, 0xd8, 0xff, 0xe0]), Buffer.from(text)]);
  return decodeAvatar('image/jpeg', data.toString('base64'));
}

test('cleanStyle keeps a name effect with the number of colors it takes', () => {
  assert.deepEqual(cleanStyle({ name: { effect: 'solid', colors: ['#FF0000', '#00ff00'] } }).name,
    { effect: 'solid', colors: ['#ff0000'] });
  assert.deepEqual(cleanStyle({ name: { effect: 'gradient', colors: ['#ff0000', '#00ff00', '#0000ff'] } }).name,
    { effect: 'gradient', colors: ['#ff0000', '#00ff00'] });
  assert.deepEqual(cleanStyle({ name: { effect: 'prism', colors: ['#111111', 'red', '#222222', '#333333'] } }).name,
    { effect: 'prism', colors: ['#111111', '#222222', '#333333'] });
  assert.equal(cleanStyle({ name: { effect: 'prism', colors: Array(9).fill('#abcdef') } }).name.colors.length, 5);
});

test('cleanStyle drops what does not fit the format', () => {
  assert.equal(cleanStyle({ name: { effect: 'gradient', colors: ['#ff0000'] } }).name, null);
  assert.equal(cleanStyle({ name: { effect: 'sparkle', colors: ['#ff0000'] } }).name, null);
  assert.equal(cleanStyle({ name: { effect: 'constructor', colors: ['#ff0000'] } }).name, null);
  assert.equal(cleanStyle({ name: 'gradient' }).name, null);
  assert.equal(cleanStyle({ theme: ['#111111'] }).theme, null);
  assert.equal(cleanStyle({ theme: ['#111111', 'url(x)'] }).theme, null);
  assert.deepEqual(cleanStyle({ theme: ['#111111', '#222222', '#333333'] }).theme, ['#111111', '#222222']);
  assert.deepEqual(cleanStyle('nope'), { name: null, theme: null, bio: '' });
});

test('cleanStyle trims the bio to a few short lines', () => {
  assert.equal(cleanStyle({ bio: '  oi\r\ntudo bem  ' }).bio, 'oi\ntudo bem');
  assert.equal(cleanStyle({ bio: 'x'.repeat(500) }).bio.length, 190);
  assert.equal(cleanStyle({ bio: 'a\nb\nc\nd\ne\nf\ng\nh' }).bio, 'a\nb\nc\nd\ne\nf');
  assert.equal(cleanStyle({ bio: 42 }).bio, '');
});

test('hasStyle tells an empty style from one worth sending', () => {
  assert.equal(hasStyle(cleanStyle(null)), false);
  assert.equal(hasStyle(cleanStyle({ bio: 'oi' })), true);
  assert.equal(hasStyle(cleanStyle({ theme: ['#111111', '#222222'] })), true);
  assert.equal(hasStyle(null), false);
});

test('the profile keeps its style and banner across restarts', (t) => {
  const file = scratchFile(t);
  const first = new ProfileStore(file, 'Rafa').load();
  assert.equal(first.setStyle({ name: { effect: 'neon', colors: ['#19b8b0'] }, bio: 'oi' }), true);
  assert.equal(first.setStyle({ name: { effect: 'neon', colors: ['#19B8B0'] }, bio: 'oi ' }), false);
  const banner = jpeg('banner');
  first.setBanner(banner);

  const again = new ProfileStore(file, 'Rafa').load();
  assert.deepEqual(again.style, { name: { effect: 'neon', colors: ['#19b8b0'] }, theme: null, bio: 'oi' });
  assert.equal(again.bannerHash, banner.hash);
  assert.match(again.bannerUrl(), /^data:image\/jpeg;base64,/);
  assert.equal(again.memberId, first.memberId);

  assert.equal(again.removeBanner(), true);
  assert.equal(again.removeBanner(), false);
  assert.equal(new ProfileStore(file, 'Rafa').load().bannerUrl(), null);
});
