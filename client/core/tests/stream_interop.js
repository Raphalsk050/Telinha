'use strict';

// The stream manager of the Electron app and the Rust hub in the same voice room, on the real
// message servers, each with a real engine: Rust shares a monitor and the Electron side watches
// it, then the Electron side shares and Rust watches. No window opens: both viewers are headless.
//
//   node client/core/tests/stream_interop.js <monitor handle>
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const readline = require('node:readline');
const { spawn } = require('node:child_process');

// The viewer of the Electron side runs without a window for this test.
const engine = require('../../../desktop/src/telinha-process');
const watchArgs = engine.buildWatchArgs;
engine.buildWatchArgs = (options) => [...watchArgs({ ...options, audio: false }), '--headless'];

const { ServerStore } = require('../../../desktop/src/servers');
const { Signaling } = require('../../../desktop/src/signaling');
const { StreamManager } = require('../../../desktop/src/streams');

const monitor = Number(process.argv[2]);
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'telinha-stream-'));
const servers = new ServerStore(path.join(scratch, 'node-servers.json')).load();
const memberId = 'e'.repeat(32);
const seen = {
  rustLive: false, nodeWatching: false, nodePresented: 0, rustViewerConnected: false, nodeLive: false,
  rustWatching: false, rustPresented: 0, rustSent: 0, rustEndedClean: null,
};
let ready = null;
let node = null;
let streams = null;
let watched = false;
let wentLive = false;

function start() {
  servers.join(ready.invite, { memberId, name: 'Node' });
  node = new Signaling({
    store: { list: () => servers.list().map(({ id }) => ({ id })), keysFor: (id) => servers.keysFor(id) },
    profileName: 'Node',
    presenceFor: (id) => ({
      memberId,
      meta: servers.meta(id),
      voice: {
        roomId: ready.room,
        mic: true,
        live: Boolean(streams && streams.outgoing && streams.outgoing.state === 'live'),
        liveName: 'Monitor do Node',
        v: 2,
      },
    }),
  });
  streams = new StreamManager({
    exe: () => engine.resolveTelinhaExe(),
    publish: (spaceId, type, body) => node.publish(spaceId, type, body),
    selfId: node.instanceId,
    ownerPid: process.pid,
  });
  // The preview of the own screen needs the page of the Electron app, which is not here.
  streams.watchOwnScreen = () => {};
  let lastLive = false;
  streams.on('changed', (snapshot) => {
    const live = Boolean(snapshot.outgoing && snapshot.outgoing.state === 'live');
    if (live !== lastLive) {
      lastLive = live;
      seen.nodeLive = seen.nodeLive || live;
      node.publishPresence(true, ready.server);
    }
    const entry = snapshot.incoming.find((item) => item.sharerId === ready.instance);
    if (entry && entry.state === 'live') {
      seen.nodeWatching = true;
      if (!wentLive) {
        wentLive = true;
        streams.goLive({
          spaceId: ready.server,
          roomId: ready.room,
          share: { targetKind: 'monitor', targetHandle: monitor, audio: 'none' },
          targetName: 'Monitor do Node',
        });
      }
    }
  });
  streams.on('event', ({ source, event }) => {
    if (source !== 'outgoing' && event.event === 'stats') {
      seen.nodePresented = Math.max(seen.nodePresented, event.presented || 0);
    }
  });
  node.on('message', ({ spaceId, message }) => {
    if (message.type.startsWith('stream-')) {
      streams.handleMessage(spaceId, message);
    }
  });
  node.on('peers', ({ spaceId, peers }) => {
    streams.reconcile(spaceId, ready.room, peers);
    const rust = peers.find((peer) => peer.id === ready.instance);
    if (rust && rust.voice && rust.voice.live && !watched) {
      watched = true;
      seen.rustLive = true;
      streams.watch({ spaceId, roomId: ready.room, sharerId: rust.id, name: 'Rust', mode: 'native' });
    }
  });
  node.start();
}

const exe = path.join(__dirname, '..', '..', 'target', 'debug', 'examples', 'stream_probe.exe');
const rust = spawn(exe, [path.join(scratch, 'rust'), String(monitor), process.argv[3] ?? '55'], {
  stdio: ['ignore', 'pipe', 'inherit'],
  env: { ...process.env, TELINHA_HEADLESS_VIEWER: '1' },
});
readline.createInterface({ input: rust.stdout }).on('line', (line) => {
  let event;
  try {
    event = JSON.parse(line);
  } catch {
    return;
  }
  if (event.event === 'ready') {
    ready = event;
    start();
  } else if (event.event === 'streams') {
    if (event.outgoing && event.outgoing.viewers.includes('Connected')) {
      seen.rustViewerConnected = true;
    }
    if (event.incoming.some((entry) => entry.phase === 'Live')) {
      seen.rustWatching = true;
    }
  } else if (event.event === 'stats' && event.source === 'incoming') {
    seen.rustPresented = Math.max(seen.rustPresented, event.presented || 0);
  } else if (event.event === 'stats' && event.source === 'outgoing') {
    seen.rustSent = Math.max(seen.rustSent, event.frames || 0);
  } else if (event.event === 'outgoing-ended') {
    seen.rustEndedClean = event.code === 0 && event.stopped === true;
  } else if (event.event === 'go-live' || event.event === 'watch') {
    console.log(`rust: ${JSON.stringify(event)}`);
  }
});
rust.on('exit', (code) => {
  setTimeout(async () => {
    if (streams) {
      streams.stopAll();
    }
    await new Promise((resolve) => setTimeout(resolve, 3000));
    if (node) {
      await node.stop();
    }
    fs.rmSync(scratch, { recursive: true, force: true });
    console.log(JSON.stringify(seen, null, 2));
    const ok = seen.rustLive && seen.nodeWatching && seen.nodePresented > 0 && seen.rustViewerConnected
      && seen.nodeLive && seen.rustWatching && seen.rustPresented > 0 && seen.rustSent > 0 && seen.rustEndedClean === true;
    console.log(ok ? 'ok: as transmissoes cruzam entre o Rust e o app Electron' : `falhou (rust saiu com ${code})`);
    process.exit(ok ? 0 : 1);
  }, 1000);
});
