'use strict';

// Minimal TURN server over UDP (RFC 5766 with RFC 5389 long-term credentials): Binding, Allocate,
// Refresh, CreatePermission, ChannelBind, Send and ChannelData. It also delivers between two of its
// own allocations without leaving the machine, because the relayed addresses carry the public IP and
// most home routers do not loop traffic sent to their own public address back inside.

const crypto = require('node:crypto');
const dgram = require('node:dgram');

const MAGIC_COOKIE = 0x2112a442;
const METHOD = { binding: 0x001, allocate: 0x003, refresh: 0x004, send: 0x006, data: 0x007, permission: 0x008, channelBind: 0x009 };
const CLASS = { request: 0x000, indication: 0x010, success: 0x100, error: 0x110 };
const ATTR = {
  username: 0x0006, integrity: 0x0008, errorCode: 0x0009, channelNumber: 0x000c, lifetime: 0x000d,
  peer: 0x0012, data: 0x0013, realm: 0x0014, nonce: 0x0015, relayed: 0x0016, transport: 0x0019,
  mapped: 0x0020, software: 0x8022, fingerprint: 0x8028,
};
const DEFAULT_LIFETIME_S = 600;
const MAX_LIFETIME_S = 3600;
const PERMISSION_S = 300;
const CHANNEL_S = 600;
const NONCE_TTL_MS = 60 * 60 * 1000;
const SOFTWARE = 'telinha-relay';

function key(address, port) {
  return `${address}:${port}`;
}

function parseMessage(buffer) {
  if (buffer.length < 20 || (buffer[0] & 0xc0) !== 0 || buffer.readUInt32BE(4) !== MAGIC_COOKIE) {
    return null;
  }
  const length = buffer.readUInt16BE(2);
  if (20 + length > buffer.length) {
    return null;
  }
  const type = buffer.readUInt16BE(0);
  const message = {
    method: (type & 0x000f) | ((type & 0x00e0) >> 1) | ((type & 0x3e00) >> 2),
    klass: type & 0x0110,
    id: buffer.subarray(8, 20),
    attrs: new Map(),
    integrityAt: -1,
    raw: buffer.subarray(0, 20 + length),
  };
  for (let at = 20; at + 4 <= 20 + length;) {
    const attrType = buffer.readUInt16BE(at);
    const attrLength = buffer.readUInt16BE(at + 2);
    if (at + 4 + attrLength > 20 + length) {
      return null;
    }
    if (attrType === ATTR.integrity) {
      message.integrityAt = at;
    }
    if (!message.attrs.has(attrType)) {
      message.attrs.set(attrType, buffer.subarray(at + 4, at + 4 + attrLength));
    }
    at += 4 + attrLength + ((4 - (attrLength % 4)) % 4);
  }
  return message;
}

function xorAddress(value, id) {
  if (!value || value.length < 8 || value[1] !== 0x01) {
    return null;
  }
  const port = value.readUInt16BE(2) ^ (MAGIC_COOKIE >>> 16);
  const raw = (value.readUInt32BE(4) ^ MAGIC_COOKIE) >>> 0;
  return { address: [24, 16, 8, 0].map((shift) => (raw >>> shift) & 0xff).join('.'), port };
}

function encodeXorAddress(address, port) {
  const value = Buffer.alloc(8);
  value[1] = 0x01;
  value.writeUInt16BE(port ^ (MAGIC_COOKIE >>> 16), 2);
  const raw = address.split('.').reduce((sum, part) => ((sum << 8) | Number(part)) >>> 0, 0);
  value.writeUInt32BE((raw ^ MAGIC_COOKIE) >>> 0, 4);
  return value;
}

function buildMessage(method, klass, id, attrs, integrityKey) {
  const parts = [];
  for (const [type, value] of attrs) {
    const header = Buffer.alloc(4);
    header.writeUInt16BE(type, 0);
    header.writeUInt16BE(value.length, 2);
    parts.push(header, value, Buffer.alloc((4 - (value.length % 4)) % 4));
  }
  let body = Buffer.concat(parts);
  const header = Buffer.alloc(20);
  const type = (method & 0x000f) | ((method & 0x0070) << 1) | ((method & 0x0f80) << 2) | klass;
  header.writeUInt16BE(type, 0);
  header.writeUInt32BE(MAGIC_COOKIE, 4);
  id.copy(header, 8);
  if (integrityKey) {
    header.writeUInt16BE(body.length + 24, 2);
    const hmac = crypto.createHmac('sha1', integrityKey).update(Buffer.concat([header, body])).digest();
    const attr = Buffer.alloc(4);
    attr.writeUInt16BE(ATTR.integrity, 0);
    attr.writeUInt16BE(20, 2);
    body = Buffer.concat([body, attr, hmac]);
  }
  header.writeUInt16BE(body.length, 2);
  return Buffer.concat([header, body]);
}

function errorValue(code, reason) {
  const text = Buffer.from(reason, 'utf8');
  const value = Buffer.alloc(4 + text.length);
  value[2] = Math.floor(code / 100);
  value[3] = code % 100;
  text.copy(value, 4);
  return value;
}

function u32(value) {
  const buffer = Buffer.alloc(4);
  buffer.writeUInt32BE(value >>> 0, 0);
  return buffer;
}

class TurnServer {
  constructor({ listenIp, port = 3478, relayIp, externalIp, minPort, maxPort, realm = 'telinha', users, log = () => {} }) {
    Object.assign(this, { listenIp, port, relayIp, externalIp, minPort, maxPort, realm, log });
    this.keys = new Map(Object.entries(users).map(([user, password]) => [
      user, crypto.createHash('md5').update(`${user}:${realm}:${password}`).digest(),
    ]));
    this.allocations = new Map(); // client "ip:port" -> allocation
    this.byRelayPort = new Map(); // relay port -> allocation
    this.nonceSecret = crypto.randomBytes(16);
  }

  start() {
    this.socket = dgram.createSocket('udp4');
    this.socket.on('message', (data, rinfo) => this.onClientPacket(data, rinfo));
    this.socket.on('error', (error) => this.log(`erro no socket principal: ${error.message}`));
    return new Promise((resolve) => this.socket.bind(this.port, this.listenIp, resolve));
  }

  // A nonce carries its own timestamp and a MAC, so the server needs no table of issued nonces.
  makeNonce() {
    const stamp = Date.now().toString(16);
    const mac = crypto.createHmac('sha256', this.nonceSecret).update(stamp).digest('hex').slice(0, 16);
    return `${stamp}-${mac}`;
  }

  nonceValid(nonce) {
    const [stamp, mac] = String(nonce).split('-');
    const expected = crypto.createHmac('sha256', this.nonceSecret).update(String(stamp)).digest('hex').slice(0, 16);
    return mac === expected && Date.now() - parseInt(stamp, 16) < NONCE_TTL_MS;
  }

  reply(rinfo, message) {
    this.socket.send(message, rinfo.port, rinfo.address);
  }

  fail(rinfo, request, code, reason, extra = []) {
    this.reply(rinfo, buildMessage(request.method, CLASS.error, request.id,
      [[ATTR.errorCode, errorValue(code, reason)], ...extra, [ATTR.software, Buffer.from(SOFTWARE)]]));
  }

  // Long-term credential check. Returns the HMAC key, or null after answering with the error.
  authenticate(rinfo, request) {
    const username = request.attrs.get(ATTR.username);
    const nonce = request.attrs.get(ATTR.nonce);
    const challenge = [[ATTR.realm, Buffer.from(this.realm)], [ATTR.nonce, Buffer.from(this.makeNonce())]];
    if (request.integrityAt < 0 || !username || !nonce) {
      this.fail(rinfo, request, 401, 'Unauthorized', challenge);
      return null;
    }
    if (!this.nonceValid(nonce.toString())) {
      this.fail(rinfo, request, 438, 'Stale Nonce', challenge);
      return null;
    }
    const hmacKey = this.keys.get(username.toString());
    if (!hmacKey) {
      this.fail(rinfo, request, 401, 'Unauthorized', challenge);
      return null;
    }
    const signed = Buffer.from(request.raw.subarray(0, request.integrityAt));
    signed.writeUInt16BE(request.integrityAt - 20 + 24, 2);
    const expected = crypto.createHmac('sha1', hmacKey).update(signed).digest();
    const got = request.raw.subarray(request.integrityAt + 4, request.integrityAt + 24);
    if (got.length !== 20 || !crypto.timingSafeEqual(expected, got)) {
      this.fail(rinfo, request, 401, 'Unauthorized', challenge);
      return null;
    }
    return hmacKey;
  }

  onClientPacket(data, rinfo) {
    // ChannelData: first two bits 01.
    if (data.length >= 4 && (data[0] & 0xc0) === 0x40) {
      const allocation = this.allocations.get(key(rinfo.address, rinfo.port));
      if (!allocation) {
        return;
      }
      const number = data.readUInt16BE(0);
      const length = data.readUInt16BE(2);
      const peer = allocation.channels.get(number);
      if (peer && 4 + length <= data.length && this.permitted(allocation, peer.address)) {
        this.relayOut(allocation, data.subarray(4, 4 + length), peer);
      }
      return;
    }
    const message = parseMessage(data);
    if (!message) {
      return;
    }
    if (message.method === METHOD.binding && message.klass === CLASS.request) {
      this.reply(rinfo, buildMessage(METHOD.binding, CLASS.success, message.id,
        [[ATTR.mapped, encodeXorAddress(rinfo.address, rinfo.port)]]));
      return;
    }
    const allocation = this.allocations.get(key(rinfo.address, rinfo.port));
    if (message.method === METHOD.send && message.klass === CLASS.indication) {
      const peer = xorAddress(message.attrs.get(ATTR.peer), message.id);
      const payload = message.attrs.get(ATTR.data);
      if (allocation && peer && payload && this.permitted(allocation, peer.address)) {
        this.relayOut(allocation, payload, peer);
      }
      return;
    }
    if (message.klass !== CLASS.request) {
      return;
    }
    const hmacKey = this.authenticate(rinfo, message);
    if (!hmacKey) {
      return;
    }
    const ok = (attrs) => this.reply(rinfo, buildMessage(message.method, CLASS.success, message.id,
      [...attrs, [ATTR.software, Buffer.from(SOFTWARE)]], hmacKey));
    switch (message.method) {
      case METHOD.allocate:
        this.allocate(rinfo, message, hmacKey, allocation, ok);
        break;
      case METHOD.refresh:
        if (!allocation) {
          this.fail(rinfo, message, 437, 'Allocation Mismatch');
          break;
        }
        this.refresh(allocation, message, ok);
        break;
      case METHOD.permission:
        if (!allocation) {
          this.fail(rinfo, message, 437, 'Allocation Mismatch');
          break;
        }
        for (const [type, value] of [...this.peerAttrs(message.raw)]) {
          const peer = type === ATTR.peer ? xorAddress(value, message.id) : null;
          if (peer) {
            allocation.permissions.set(peer.address, Date.now() + PERMISSION_S * 1000);
          }
        }
        ok([]);
        break;
      case METHOD.channelBind: {
        const numberValue = message.attrs.get(ATTR.channelNumber);
        const peer = xorAddress(message.attrs.get(ATTR.peer), message.id);
        const number = numberValue ? numberValue.readUInt16BE(0) : 0;
        if (!allocation || !peer || number < 0x4000 || number > 0x7ffe) {
          this.fail(rinfo, message, allocation ? 400 : 437, allocation ? 'Bad Request' : 'Allocation Mismatch');
          break;
        }
        allocation.channels.set(number, peer);
        allocation.channelOf.set(key(peer.address, peer.port), number);
        allocation.permissions.set(peer.address, Date.now() + CHANNEL_S * 1000);
        ok([]);
        break;
      }
      default:
        this.fail(rinfo, message, 400, 'Bad Request');
    }
  }

  // Every XOR-PEER-ADDRESS in a CreatePermission (it may carry several).
  * peerAttrs(raw) {
    const length = raw.readUInt16BE(2);
    for (let at = 20; at + 4 <= 20 + length;) {
      const type = raw.readUInt16BE(at);
      const attrLength = raw.readUInt16BE(at + 2);
      yield [type, raw.subarray(at + 4, at + 4 + attrLength)];
      at += 4 + attrLength + ((4 - (attrLength % 4)) % 4);
    }
  }

  allocate(rinfo, message, hmacKey, existing, ok) {
    if (existing) {
      if (existing.transactionId.equals(message.id)) {
        ok(existing.successAttrs);
        return;
      }
      this.fail(rinfo, message, 437, 'Allocation Mismatch');
      return;
    }
    const transport = message.attrs.get(ATTR.transport);
    if (!transport || transport[0] !== 17) {
      this.fail(rinfo, message, 442, 'Unsupported Transport Protocol');
      return;
    }
    const port = this.freePort();
    if (!port) {
      this.fail(rinfo, message, 508, 'Insufficient Capacity');
      return;
    }
    const relay = dgram.createSocket('udp4');
    const allocation = {
      client: rinfo, transactionId: Buffer.from(message.id), relay, port,
      permissions: new Map(), channels: new Map(), channelOf: new Map(), timer: null, successAttrs: [],
    };
    this.byRelayPort.set(port, allocation);
    relay.on('message', (data, from) => this.onPeerPacket(allocation, data, from));
    relay.on('error', (error) => this.log(`erro no relay ${port}: ${error.message}`));
    relay.bind(port, this.relayIp, () => {
      this.allocations.set(key(rinfo.address, rinfo.port), allocation);
      const lifetime = this.lifetimeFrom(message);
      this.arm(allocation, lifetime);
      allocation.successAttrs = [
        [ATTR.relayed, encodeXorAddress(this.externalIp, port)],
        [ATTR.mapped, encodeXorAddress(rinfo.address, rinfo.port)],
        [ATTR.lifetime, u32(lifetime)],
      ];
      ok(allocation.successAttrs);
      this.log(`alocacao ${port} aberta (${this.allocations.size} ativas)`);
    });
  }

  lifetimeFrom(message) {
    const value = message.attrs.get(ATTR.lifetime);
    const asked = value && value.length === 4 ? value.readUInt32BE(0) : DEFAULT_LIFETIME_S;
    return Math.min(Math.max(asked, 0), MAX_LIFETIME_S);
  }

  refresh(allocation, message, ok) {
    const lifetime = this.lifetimeFrom(message);
    if (lifetime === 0) {
      this.close(allocation);
    } else {
      this.arm(allocation, lifetime);
    }
    ok([[ATTR.lifetime, u32(lifetime)]]);
  }

  arm(allocation, seconds) {
    clearTimeout(allocation.timer);
    allocation.timer = setTimeout(() => this.close(allocation), seconds * 1000);
    allocation.timer.unref();
  }

  close(allocation) {
    clearTimeout(allocation.timer);
    this.allocations.delete(key(allocation.client.address, allocation.client.port));
    this.byRelayPort.delete(allocation.port);
    try {
      allocation.relay.close();
    } catch {
      // ja fechado
    }
    this.log(`alocacao ${allocation.port} fechada (${this.allocations.size} ativas)`);
  }

  freePort() {
    const ports = [];
    for (let port = this.minPort; port <= this.maxPort; port += 1) {
      if (!this.byRelayPort.has(port)) {
        ports.push(port);
      }
    }
    return ports.length ? ports[crypto.randomInt(ports.length)] : 0;
  }

  permitted(allocation, address) {
    const until = allocation.permissions.get(address);
    return Boolean(until && until > Date.now());
  }

  // Sends client data to a peer. A peer that is another allocation here is reached directly.
  relayOut(allocation, payload, peer) {
    const local = peer.address === this.externalIp ? this.byRelayPort.get(peer.port) : null;
    if (local) {
      this.onPeerPacket(local, payload, { address: this.externalIp, port: allocation.port });
      return;
    }
    allocation.relay.send(payload, peer.port, peer.address);
  }

  onPeerPacket(allocation, data, from) {
    let { address, port } = from;
    // A packet from one of our own relay sockets arrives from the LAN address.
    if (address === this.relayIp && this.byRelayPort.has(port)) {
      address = this.externalIp;
    }
    if (!this.permitted(allocation, address)) {
      return;
    }
    const { client } = allocation;
    const number = allocation.channelOf.get(key(address, port));
    if (number !== undefined) {
      const header = Buffer.alloc(4);
      header.writeUInt16BE(number, 0);
      header.writeUInt16BE(data.length, 2);
      const padding = Buffer.alloc((4 - (data.length % 4)) % 4);
      this.socket.send(Buffer.concat([header, data, padding]), client.port, client.address);
      return;
    }
    this.socket.send(buildMessage(METHOD.data, CLASS.indication, crypto.randomBytes(12),
      [[ATTR.peer, encodeXorAddress(address, port)], [ATTR.data, data]]), client.port, client.address);
  }
}

module.exports = { TurnServer, parseMessage, buildMessage, encodeXorAddress, xorAddress, METHOD, CLASS, ATTR };
