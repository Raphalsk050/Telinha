'use strict';

// TURN relay for Telinha calls and streams that cannot connect directly. Runs on a PC whose router
// forwards UDP port 3478 and the relay range to it. Settings live in relay.json next to this file
// (not in git): { "lanIp": "192.168.1.9", "username": "...", "password": "...",
//   "port": 3478, "minPort": 49160, "maxPort": 49199 }.
// The public address is looked up by STUN and followed when it changes, so the relayed addresses
// handed to clients stay right on a dynamic IP.

const crypto = require('node:crypto');
const dgram = require('node:dgram');
const fs = require('node:fs');
const path = require('node:path');
const { TurnServer } = require('./turn');

const CONFIG_PATH = path.join(__dirname, 'relay.json');
const STUN_SERVERS = [['stun.cloudflare.com', 3478], ['stun.l.google.com', 19302]];
const PUBLIC_IP_CHECK_MS = 5 * 60 * 1000;
const MAGIC_COOKIE = 0x2112a442;

function log(line) {
  console.log(`${new Date().toISOString()} ${line}`);
}

function loadConfig() {
  const config = JSON.parse(fs.readFileSync(CONFIG_PATH, 'utf8'));
  for (const key of ['lanIp', 'username', 'password']) {
    if (typeof config[key] !== 'string' || config[key] === '') {
      throw new Error(`relay.json precisa de "${key}"`);
    }
  }
  return {
    port: 3478, minPort: 49160, maxPort: 49199, realm: 'telinha', ...config,
  };
}

// One STUN binding request; resolves the public IPv4 address or null.
function stunPublicIp(host, port) {
  return new Promise((resolve) => {
    const socket = dgram.createSocket('udp4');
    const id = crypto.randomBytes(12);
    const request = Buffer.alloc(20);
    request.writeUInt16BE(0x0001, 0);
    request.writeUInt32BE(MAGIC_COOKIE, 4);
    id.copy(request, 8);
    const timer = setTimeout(() => finish(null), 2000);
    function finish(value) {
      clearTimeout(timer);
      socket.close();
      resolve(value);
    }
    socket.on('error', () => finish(null));
    socket.on('message', (data) => {
      if (data.length < 20 || !data.subarray(8, 20).equals(id)) {
        return;
      }
      for (let at = 20; at + 4 <= data.length;) {
        const type = data.readUInt16BE(at);
        const length = data.readUInt16BE(at + 2);
        if (type === 0x0020 && length >= 8 && data[at + 5] === 0x01) {
          const raw = data.readUInt32BE(at + 8) ^ MAGIC_COOKIE;
          finish([24, 16, 8, 0].map((shift) => (raw >>> shift) & 0xff).join('.'));
          return;
        }
        at += 4 + length + ((4 - (length % 4)) % 4);
      }
      finish(null);
    });
    socket.send(request, port, host);
  });
}

async function publicIp() {
  for (const [host, port] of STUN_SERVERS) {
    const ip = await stunPublicIp(host, port);
    if (ip) {
      return ip;
    }
  }
  return null;
}

async function main() {
  const config = loadConfig();
  const external = await publicIp();
  if (!external) {
    throw new Error('nenhum servidor STUN respondeu, sem endereco publico para anunciar');
  }
  const server = new TurnServer({
    listenIp: config.lanIp,
    relayIp: config.lanIp,
    externalIp: external,
    port: config.port,
    minPort: config.minPort,
    maxPort: config.maxPort,
    realm: config.realm,
    users: { [config.username]: config.password, ...(config.extraUsers || {}) },
    log: process.env.RELAY_DEBUG ? log : () => {},
  });
  await server.start();
  log(`relay em ${config.lanIp}:${config.port}, portas ${config.minPort}-${config.maxPort}, anunciando o endereco publico atual`);

  setInterval(async () => {
    const current = await publicIp();
    if (current && current !== server.externalIp) {
      server.externalIp = current;
      log('o endereco publico mudou, as proximas alocacoes ja usam o novo');
    }
  }, PUBLIC_IP_CHECK_MS).unref();
}

main().catch((error) => {
  log(`erro: ${error.message}`);
  process.exit(1);
});
