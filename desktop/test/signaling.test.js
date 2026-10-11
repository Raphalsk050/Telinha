'use strict';

const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');

const { ContactStore } = require('../src/contacts');
const { Signaling } = require('../src/signaling');

const INVITE = 'TELINHA1.VExTMQEAAgAxCwAAdj0wDQpvPS0gMjYwMTM4ODI0';
const ANSWER = 'TELINHA1.VExTMQIBAgAxCwAAdj0wDQpvPS0gMzcxNDg5OTM1';

class FakeBroker {
  constructor() {
    this.clients = new Set();
    this.published = [];
  }

  connect() {
    const broker = this;
    const client = new EventEmitter();
    client.topics = new Set();
    client.subscribe = (topics) => {
      for (const topic of [].concat(topics)) {
        client.topics.add(topic);
      }
    };
    client.unsubscribe = (topic) => client.topics.delete(topic);
    client.publish = (topic, payload) => {
      broker.published.push({ topic, payload });
      setImmediate(() => {
        for (const other of broker.clients) {
          if (other.topics.has(topic)) {
            other.emit('message', topic, Buffer.from(payload));
          }
        }
      });
    };
    client.end = (_force, _options, done) => {
      broker.clients.delete(client);
      setImmediate(() => {
        client.emit('close');
        if (done) {
          done();
        }
      });
    };
    broker.clients.add(client);
    setImmediate(() => client.emit('connect'));
    return client;
  }
}

function pairedStore() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'telinha-signaling-'));
  const store = new ContactStore(path.join(directory, 'contacts.json'));
  const { contact } = store.saveFromCodes(INVITE, ANSWER, 'Contato');
  return { store, contactId: contact.id };
}

function nextEvent(emitter, name) {
  return new Promise((resolve) => emitter.once(name, resolve));
}

test('two apps see each other online and exchange encrypted requests', async () => {
  const broker = new FakeBroker();
  const connect = () => broker.connect();

  const left = pairedStore();
  const right = pairedStore();
  assert.equal(left.contactId, right.contactId);

  const alice = new Signaling({ store: left.store, profileName: 'Alice', brokers: ['fake://a'], connect });
  const bob = new Signaling({ store: right.store, profileName: 'Bob', brokers: ['fake://a'], connect });

  const aliceSeesBob = nextEvent(alice, 'presence');
  const bobName = nextEvent(alice, 'peer-name');
  alice.start();
  bob.start();

  assert.deepEqual(await aliceSeesBob, { contactId: left.contactId, spaceId: left.contactId, online: true });
  assert.deepEqual(await bobName, { contactId: left.contactId, spaceId: left.contactId, name: 'Bob' });

  const received = nextEvent(bob, 'message');
  assert.equal(alice.publish(left.contactId, 'request', { sessionId: 's1', role: 'share' }), true);
  const { contactId, message } = await received;
  assert.equal(contactId, right.contactId);
  assert.equal(message.type, 'request');
  assert.equal(message.role, 'share');
  assert.equal(message.sessionId, 's1');

  for (const { payload } of broker.published) {
    assert.equal(String(payload).includes('share'), false);
  }

  const offline = nextEvent(alice, 'presence');
  await bob.stop();
  assert.deepEqual(await offline, { contactId: left.contactId, spaceId: left.contactId, online: false });
  await alice.stop();
});

test('a message larger than a server accepts goes only through the others', async () => {
  const small = new FakeBroker();
  const large = new FakeBroker();
  const { store, contactId } = pairedStore();
  const app = new Signaling({
    store,
    profileName: 'Solo',
    brokers: [{ url: 'fake://small', maxBytes: 2000 }, 'fake://large'],
    connect: (url) => (url === 'fake://small' ? small : large).connect(),
  });
  app.start();
  await nextEvent(app, 'status');
  await new Promise((resolve) => setTimeout(resolve, 20));
  small.published.length = 0;
  large.published.length = 0;

  assert.equal(app.publish(contactId, 'request', { blob: 'x'.repeat(5000) }), true);
  assert.equal(small.published.length, 0);
  assert.equal(large.published.length, 1);

  assert.equal(app.publish(contactId, 'request', { blob: 'x' }), true);
  assert.equal(small.published.length, 1);
  assert.equal(large.published.length, 2);
  await app.stop();

  const alone = new Signaling({
    store, profileName: 'Solo', brokers: [{ url: 'fake://small', maxBytes: 2000 }], connect: () => small.connect(),
  });
  alone.start();
  await nextEvent(alone, 'status');
  assert.equal(alone.publish(contactId, 'request', { blob: 'x'.repeat(5000) }), false);
  await alone.stop();
});

test('presence carries the call version, and an app that says none counts as the first', async () => {
  const broker = new FakeBroker();
  const connect = () => broker.connect();
  const left = pairedStore();
  const right = pairedStore();
  const alice = new Signaling({
    store: left.store,
    profileName: 'Alice',
    brokers: ['fake://a'],
    connect,
    presenceFor: () => ({ voice: { roomId: 'sala', v: 2 } }),
  });
  const bob = new Signaling({
    store: right.store,
    profileName: 'Bob',
    brokers: ['fake://a'],
    connect,
    presenceFor: () => ({ voice: { roomId: 'sala' } }),
  });

  const bobSeesAlice = nextEvent(bob, 'peers');
  const aliceSeesBob = nextEvent(alice, 'peers');
  alice.start();
  bob.start();
  assert.equal((await bobSeesAlice).peers[0].voice.v, 2);
  assert.equal((await aliceSeesBob).peers[0].voice.v, 1);
  await alice.stop();
  await bob.stop();
});

test('messages sent by the same app are ignored', async () => {
  const broker = new FakeBroker();
  const { store, contactId } = pairedStore();
  const lonely = new Signaling({
    store, profileName: 'Solo', brokers: ['fake://a'], connect: () => broker.connect(),
  });

  let echoed = false;
  lonely.on('message', () => {
    echoed = true;
  });
  lonely.start();
  await nextEvent(lonely, 'status');
  lonely.publish(contactId, 'request', { sessionId: 'eco', role: 'watch' });
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(echoed, false);
  await lonely.stop();
});
