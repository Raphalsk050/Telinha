'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');

const { ServerStore, sanitizeRelay } = require('../src/servers');

function freshStore() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'telinha-servers-'));
  return new ServerStore(path.join(dir, 'servers.json')).load();
}

test('sanitizeRelay keeps TURN urls and drops anything else', () => {
  assert.deepEqual(sanitizeRelay({ urls: 'turn:a.example:3478', username: 'u', credential: 'p' }),
    { urls: ['turn:a.example:3478'], username: 'u', credential: 'p' });
  assert.deepEqual(sanitizeRelay({ urls: 'turn:a.example:3478, https://x,turn:192.168.1.9:3478' }).urls,
    ['turn:a.example:3478', 'turn:192.168.1.9:3478']);
  assert.equal(sanitizeRelay({ urls: 'https://a.example' }), null);
  assert.equal(sanitizeRelay(null), null);
});

test('the relay set by one member reaches another through the server meta', () => {
  const owner = freshStore();
  const member = freshStore();
  const { id } = owner.create('Teste', null);
  member.join(owner.inviteCode(id), null);

  const relay = { urls: ['turn:a.example:3478'], username: 'u', credential: 'p' };
  assert.equal(owner.setRelay(id, relay), true);
  assert.equal(owner.setRelay(id, relay), false);
  assert.equal(member.applyMeta(id, owner.meta(id)).changed, true);
  assert.deepEqual(member.view(id).relay, relay);

  // A member on an older version sends no relay and must not wipe it.
  const old = { ...owner.meta(id) };
  delete old.relay;
  delete old.relayVersion;
  assert.equal(member.applyMeta(id, old).mineNewer, false);
  assert.deepEqual(member.view(id).relay, relay);

  owner.setRelay(id, null);
  member.applyMeta(id, owner.meta(id));
  assert.equal(member.view(id).relay, null);
});
