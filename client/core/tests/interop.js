'use strict';

// The JavaScript signaling of the Electron app and the Rust one, face to face on the real message
// servers with a key made up for this run. Checks that each side sees the other, that a chat
// message crosses both ways, that a large message crosses, and that the goodbye is heard.
//
//   node client/core/tests/interop.js
const crypto = require('node:crypto');
const path = require('node:path');
const readline = require('node:readline');
const { spawn } = require('node:child_process');
const { contactKeys } = require('../../../desktop/src/contacts');
const { Signaling } = require('../../../desktop/src/signaling');

const secret = crypto.randomBytes(32);
const keys = contactKeys(secret);
const spaces = { list: () => [{ id: keys.id }], keysFor: (id) => (id === keys.id ? keys : null) };
const seen = {
  rustSawNode: false, nodeSawRust: false, nodeGotChat: null, rustGotChat: null, rustGotLarge: 0,
  nodeGotLarge: 0, rustAck: null, rustStyleReached: false, rustGoodbye: false,
};

const node = new Signaling({
  store: spaces,
  profileName: 'Node',
  presenceFor: () => ({ memberId: 'e'.repeat(32), voice: { roomId: 'dm', mic: true, v: 2 } }),
});
node.on('peers', ({ peers }) => {
  const rust = peers.find((peer) => peer.name === 'Rust');
  if (rust) {
    seen.nodeSawRust = true;
    seen.rustStyleReached = Boolean(rust.style && rust.style.bio === 'feito em Rust') && rust.memberId === 'f'.repeat(32);
    node.publish(keys.id, 'chat', { messageId: 'from-node', text: 'olá do Node, ação 😀' });
    node.publish(keys.id, 'file-chunk', { transferId: 'node-transfer', index: 0, total: 1, data: 'B'.repeat(200 * 1024) });
  } else if (seen.nodeSawRust && peers.length === 0) {
    seen.rustGoodbye = true;
  }
});
node.on('message', ({ message }) => {
  if (message.type === 'chat') {
    seen.nodeGotChat = message.text;
  } else if (message.type === 'file-chunk') {
    seen.nodeGotLarge = message.data.length;
  }
});
node.start();

const exe = path.join(__dirname, '..', '..', 'target', 'debug', 'examples', 'signal_probe.exe');
const rust = spawn(exe, [keys.key.toString('hex'), keys.id, process.argv[2] ?? '25'], { stdio: ['ignore', 'pipe', 'inherit'] });
readline.createInterface({ input: rust.stdout }).on('line', (line) => {
  let event;
  try {
    event = JSON.parse(line);
  } catch {
    return;
  }
  if (event.event === 'status') {
    console.log(`rust: ligado a ${event.brokers.length} servidor(es)`);
  } else if (event.event === 'peers') {
    seen.rustSawNode = seen.rustSawNode || event.peers.some((peer) => peer.name === 'Node' && peer.voice === 'dm');
  } else if (event.event === 'message' && event.type === 'chat') {
    seen.rustGotChat = event.text;
  } else if (event.event === 'message' && event.type === 'file-chunk') {
    seen.rustGotLarge = event.size;
  } else if (event.event === 'acked') {
    seen.rustAck = event.delivered;
  } else if (event.event === 'sent') {
    console.log(`rust: enviou ${JSON.stringify(event)}`);
  }
});
rust.on('exit', (code) => {
  setTimeout(async () => {
    await node.stop();
    console.log(JSON.stringify(seen, null, 2));
    const ok = seen.rustSawNode && seen.nodeSawRust && seen.nodeGotChat === 'olá do Rust, ação 😀'
      && seen.rustGotChat === 'olá do Node, ação 😀' && seen.rustGotLarge > 200000 && seen.nodeGotLarge === 200 * 1024
      && seen.rustAck === true && seen.rustStyleReached && seen.rustGoodbye;
    console.log(ok ? 'ok: as duas pontas se entendem' : `falhou (rust saiu com ${code})`);
    process.exit(ok ? 0 : 1);
  }, 2500);
});
