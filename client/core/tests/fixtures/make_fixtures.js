'use strict';

// Writes what the JavaScript app produces for fixed inputs, for the Rust tests to compare against.
// node client/core/tests/fixtures/make_fixtures.js
const fs = require('node:fs');
const path = require('node:path');
const {
  contactKeys, deriveContactSecret, sealMessage,
} = require('../../../../desktop/src/contacts');

const INVITE = 'TELINHA1.VExTaW52aXRlLWNvZGUtZm9yLXRlc3Rz';
const ANSWER = 'TELINHA1.VExTYW5zd2VyLWNvZGUtZm9yLXRlc3Rz';
const keys = contactKeys(deriveContactSecret(INVITE, ANSWER));
const sealedAt = 1763150000000;
const sealed = sealMessage(keys.key, { type: 'chat', at: sealedAt, text: 'olá, açúcar 😀' });

fs.writeFileSync(path.join(__dirname, 'keys.json'), `${JSON.stringify({
  id: keys.id, key: keys.key.toString('hex'), sealedAt, sealed,
}, null, 2)}\n`);
