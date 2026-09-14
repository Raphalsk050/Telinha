'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');

const {
  ContactStore,
  cleanName,
  contactKeys,
  deriveContactSecret,
  openMessage,
  sealMessage,
} = require('../src/contacts');

const INVITE = 'TELINHA1.VExTMQEAAgAxCwAAdj0wDQpvPS0gMjYwMTM4ODI0';
const ANSWER = 'TELINHA1.VExTMQIBAgAxCwAAdj0wDQpvPS0gMzcxNDg5OTM1';

function temporaryStore(options) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'telinha-contacts-'));
  return new ContactStore(path.join(directory, 'contacts.json'), options);
}

test('both sides derive the same contact from the exchanged codes', () => {
  const sharer = contactKeys(deriveContactSecret(INVITE, ANSWER));
  const viewer = contactKeys(deriveContactSecret(` ${INVITE}\n`, `${ANSWER.slice(0, 10)}\n${ANSWER.slice(10)}`));
  assert.equal(sharer.id, viewer.id);
  assert.deepEqual(sharer.key, viewer.key);
  assert.notEqual(contactKeys(deriveContactSecret(ANSWER, INVITE)).id, sharer.id);
});

test('sealed messages open only with the right key and a fresh timestamp', () => {
  const { key } = contactKeys(deriveContactSecret(INVITE, ANSWER));
  const other = contactKeys(deriveContactSecret(INVITE, `${ANSWER}x`)).key;
  const message = { type: 'request', role: 'share', at: Date.now() };

  const sealed = sealMessage(key, message);
  assert.deepEqual(openMessage(key, sealed), message);
  assert.equal(openMessage(other, sealed), null);

  const tampered = Buffer.from(sealed, 'base64');
  tampered[tampered.length - 1] ^= 0xff;
  assert.equal(openMessage(key, tampered.toString('base64')), null);

  const stale = sealMessage(key, { type: 'request', at: Date.now() - 10 * 60 * 1000 });
  assert.equal(openMessage(key, stale), null);
  assert.equal(openMessage(key, 'nao e base64 de nada'), null);
});

test('the store saves contacts once, keeps secrets encrypted and survives a reload', () => {
  const encrypt = (text) => Buffer.from(text).reverse().toString('base64');
  const decrypt = (stored) => Buffer.from(stored, 'base64').reverse().toString();
  const store = temporaryStore({ encrypt, decrypt });

  const first = store.saveFromCodes(INVITE, ANSWER, '  Amigo   do  jogo ');
  assert.equal(first.created, true);
  assert.equal(first.contact.name, 'Amigo do jogo');
  assert.equal(store.saveFromCodes(INVITE, ANSWER, 'outro nome').created, false);
  assert.equal(store.list().length, 1);

  const raw = fs.readFileSync(store.filePath, 'utf8');
  const expectedSecret = deriveContactSecret(INVITE, ANSWER).toString('base64');
  assert.equal(raw.includes(expectedSecret), false);

  const reloaded = new ContactStore(store.filePath, { encrypt, decrypt }).load();
  assert.equal(reloaded.list()[0].id, first.contact.id);
  assert.deepEqual(reloaded.keysFor(first.contact.id), contactKeys(deriveContactSecret(INVITE, ANSWER)));
});

test('a custom name wins over the name the other side announces', () => {
  const store = temporaryStore();
  const { contact } = store.saveFromCodes(INVITE, ANSWER, 'Contato');

  assert.equal(store.adoptRemoteName(contact.id, 'Rafa'), true);
  assert.equal(store.get(contact.id).name, 'Rafa');
  assert.equal(store.rename(contact.id, 'Irmao'), true);
  assert.equal(store.adoptRemoteName(contact.id, 'Rafa'), false);
  assert.equal(store.get(contact.id).name, 'Irmao');

  assert.equal(store.remove(contact.id), true);
  assert.equal(store.remove(contact.id), false);
  assert.equal(store.keysFor(contact.id), null);
});

test('cleanName collapses whitespace, trims and falls back', () => {
  assert.equal(cleanName('  a   b '), 'a b');
  assert.equal(cleanName('   '), 'Contato');
  assert.equal(cleanName('x'.repeat(80)).length, 40);
});
