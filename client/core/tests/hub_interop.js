'use strict';

// The modules of the Electron app play one member of a server and the Rust hub plays another, on
// the real message servers, in a server made for this run. Checks the invite, the server meta,
// chat both ways with a picture, a file in pieces both ways, the avatar exchange and erasing.
//
//   node client/core/tests/hub_interop.js
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const readline = require('node:readline');
const { spawn } = require('node:child_process');
const { decodeAvatar } = require('../../../desktop/src/avatars');
const { ServerStore } = require('../../../desktop/src/servers');
const { Signaling } = require('../../../desktop/src/signaling');
const { CHUNK_BYTES, TransferInbox, sha256, splitFile } = require('../../../desktop/src/transfers');

const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'telinha-hub-'));
const servers = new ServerStore(path.join(scratch, 'node-servers.json')).load();
const memberId = 'e'.repeat(32);
const png = Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), crypto.randomBytes(900)]);
const avatar = decodeAvatar('image/png', png.toString('base64'));
const fileData = crypto.randomBytes(CHUNK_BYTES * 2 + 1234);
const seen = {
  joined: false, gotChannels: false, rustSawNode: false, nodeGotText: null, nodeGotFile: null, rustGotText: null,
  rustGotImage: null, rustGotFile: null, rustGotAvatar: null, rustRemoved: false, rustNotified: false,
  rustFileDelivered: null, nodeAdded: false,
};
let node = null;
let ready = null;
let sent = false;

const inbox = new TransferInbox({
  onComplete: (_spaceId, message, data) => {
    seen.nodeGotFile = { name: message.file.name, ok: sha256(data) === message.file.sha256, size: data.length };
  },
});

function startNode() {
  const result = servers.join(ready.invite, { memberId, name: 'Node' });
  seen.joined = Boolean(result && result.created && result.server.id === ready.server);
  node = new Signaling({
    store: { list: () => servers.list().map(({ id }) => ({ id })), keysFor: (id) => servers.keysFor(id) },
    profileName: 'Node',
    presenceFor: (id) => ({ memberId, avatarHash: avatar.hash, meta: servers.meta(id) }),
  });
  node.on('peer-presence', ({ spaceId, message }) => {
    if (message.meta && servers.applyMeta(spaceId, message.meta).changed) {
      seen.gotChannels = servers.view(spaceId).channels.some((channel) => channel.id === ready.channel);
    }
    if (seen.gotChannels && !sent) {
      sent = true;
      sendFromNode();
    }
  });
  node.on('message', ({ spaceId, message }) => {
    if (message.type === 'chat' && message.file) {
      inbox.addMessage(spaceId, message);
    } else if (message.type === 'chat') {
      seen.nodeGotText = { text: message.text, author: message.author, channel: message.channelId === ready.channel };
    } else if (message.type === 'file-chunk') {
      inbox.addChunk(spaceId, message);
    } else if (message.type === 'avatar-request' && message.to === node.instanceId && message.hash === avatar.hash) {
      node.publish(spaceId, 'avatar', {
        to: message.from, memberId, hash: avatar.hash, mime: avatar.mime, data: avatar.data.toString('base64'),
      });
    }
  });
  node.start();
}

function sendFromNode() {
  const base = { channelId: ready.channel, author: 'Node', memberId };
  node.publish(ready.server, 'chat', {
    ...base,
    messageId: 'node-text',
    text: 'oi do Node https://example.com',
    sentAt: Date.now(),
    image: { mime: 'image/png', width: 32, height: 16, data: png.toString('base64') },
    embed: { url: 'https://example.com', title: 'Exemplo', description: 'uma pagina', siteName: 'example' },
  });
  const transferId = crypto.randomUUID();
  const chunks = splitFile(fileData);
  chunks.forEach((chunk, index) => {
    node.publish(ready.server, 'file-chunk', { transferId, index, total: chunks.length, data: chunk.toString('base64') });
  });
  node.publish(ready.server, 'chat', {
    ...base,
    messageId: 'node-file',
    text: '',
    sentAt: Date.now(),
    file: { transferId, name: '..\\pasta\\relatorio final.pdf', mime: 'application/pdf', size: fileData.length, sha256: sha256(fileData) },
  });
  setTimeout(() => {
    node.publish(ready.server, 'chat-delete', { messageId: 'node-text', channelId: ready.channel, memberId });
    servers.addChannel(ready.server, 'text', 'Canal Do Node');
    node.publishPresence(true, ready.server);
  }, 9000);
}

const exe = path.join(__dirname, '..', '..', 'target', 'debug', 'examples', 'hub_probe.exe');
const rust = spawn(exe, [path.join(scratch, 'rust'), process.argv[2] ?? '40'], { stdio: ['ignore', 'pipe', 'inherit'] });
readline.createInterface({ input: rust.stdout }).on('line', (line) => {
  let event;
  try {
    event = JSON.parse(line);
  } catch {
    return;
  }
  if (event.event === 'ready') {
    ready = event;
    startNode();
  } else if (event.event === 'peers') {
    seen.rustSawNode = seen.rustSawNode || event.names.includes('Node');
  } else if (event.event === 'chat' && event.id === 'node-text') {
    seen.rustGotText = { text: event.text, author: event.author, embed: event.embed };
    seen.rustGotImage = event.image;
  } else if (event.event === 'chat' && event.id === 'node-file') {
    seen.rustGotFile = { name: event.file.name, ok: event.file.sha256 === sha256(fileData), size: event.file.size };
  } else if (event.event === 'picture' && !event.banner) {
    seen.rustGotAvatar = event.bytes;
  } else if (event.event === 'removed' && event.id === 'node-text') {
    seen.rustRemoved = true;
  } else if (event.event === 'update') {
    seen.rustFileDelivered = event.delivered;
  } else if (event.event === 'servers') {
    seen.nodeAdded = seen.nodeAdded || (event.channels.includes('canal-do-node') && event.members.includes('Node'));
  }
});
rust.on('exit', (code) => {
  setTimeout(async () => {
    if (node) {
      await node.stop();
    }
    fs.rmSync(scratch, { recursive: true, force: true });
    console.log(JSON.stringify(seen, null, 2));
    const ok = seen.joined && seen.gotChannels && seen.rustSawNode
      && seen.nodeGotText && seen.nodeGotText.text === 'oi do hub em Rust' && seen.nodeGotText.author === 'Rust' && seen.nodeGotText.channel
      && seen.nodeGotFile && seen.nodeGotFile.ok && seen.nodeGotFile.name === 'dados.bin' && seen.nodeGotFile.size === 600000
      && seen.rustGotText && seen.rustGotText.author === 'Node' && seen.rustGotText.embed === 'Exemplo'
      && seen.rustGotImage && seen.rustGotImage.bytes === png.length && seen.rustGotImage.width === 32
      && seen.rustGotFile && seen.rustGotFile.ok && seen.rustGotFile.name === 'relatorio final.pdf'
      && seen.rustGotAvatar === png.length && seen.rustRemoved && seen.rustFileDelivered === true && seen.nodeAdded;
    console.log(ok ? 'ok: o hub em Rust e os modulos do Electron se entendem' : `falhou (rust saiu com ${code})`);
    process.exit(ok ? 0 : 1);
  }, 2500);
});
