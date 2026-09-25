'use strict';

const crypto = require('node:crypto');
const { EventEmitter } = require('node:events');
const { openMessage, sealMessage } = require('./contacts');

const DEFAULT_BROKERS = [
  'wss://broker.hivemq.com:8884/mqtt',
  'wss://broker.emqx.io:8084/mqtt',
  'wss://test.mosquitto.org:8081',
];
const TOPIC_PREFIX = 'telinha/v1/';
const PRESENCE_INTERVAL_MS = 25000;
const PRESENCE_TTL_MS = 70000;
const RETRY_DELAY_MS = 5000;
const MAX_RETRY_DELAY_MS = 60000;
const RECONNECT_DELAY_MS = 1000;
// Cobre a folga de relogio que openMessage aceita, para a copia que chega por outro servidor
// nunca passar como mensagem nova.
const SEEN_TTL_MS = 6 * 60 * 1000;
const MAX_SEEN = 5000;
const WIDE_PAYLOAD_BYTES = 16 * 1024;
const PEER_ID_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;
const MEMBER_ID_PATTERN = /^[a-f0-9]{32}$/;

function defaultConnect(url, options) {
  return require('mqtt').connect(url, options);
}

function text(value, max) {
  return typeof value === 'string' ? value.slice(0, max) : '';
}

function describePeer(message) {
  const voice = message.voice && typeof message.voice === 'object'
    && PEER_ID_PATTERN.test(String(message.voice.roomId))
    ? {
      roomId: String(message.voice.roomId),
      mic: message.voice.mic !== false,
      camera: message.voice.camera === true,
      deaf: message.voice.deaf === true,
      live: message.voice.live === true,
      liveName: text(message.voice.liveName, 80),
    }
    : null;
  return {
    name: text(message.name, 40),
    memberId: MEMBER_ID_PATTERN.test(String(message.memberId)) ? message.memberId : null,
    voice,
  };
}

// O app fica ligado a todos os servidores de mensagens ao mesmo tempo. Quem abre o app acha os
// outros pelo primeiro servidor que responder, e duas pessoas se encontram mesmo quando cada uma
// so alcanca um servidor diferente. Toda mensagem leva um id, e a copia que chega por outro
// servidor e descartada.
class Signaling extends EventEmitter {
  constructor({
    store, spaces = store, profileName, presenceFor = null, brokers = DEFAULT_BROKERS, connect = defaultConnect,
  }) {
    super();
    this.spaces = spaces;
    this.profileName = profileName;
    this.presenceFor = presenceFor;
    this.connectClient = connect;
    this.instanceId = crypto.randomUUID();
    this.links = brokers.map((url) => ({
      url, client: null, connected: false, failures: 0, retryTimer: null,
    }));
    this.stopped = true;
    this.presence = new Map();
    this.routes = new Map();
    this.seen = new Map();
    this.presenceTimer = null;
  }

  get connected() {
    return this.links.some((link) => link.connected);
  }

  start() {
    this.stopped = false;
    for (const link of this.links) {
      this.connect(link);
    }
    this.presenceTimer = setInterval(() => {
      this.publishPresence(true);
      this.expirePresence();
      this.expireSeen();
    }, PRESENCE_INTERVAL_MS);
  }

  connect(link) {
    if (this.stopped) {
      return;
    }
    const client = this.connectClient(link.url, {
      clientId: `telinha-${this.instanceId.slice(0, 8)}-${Date.now().toString(36)}`,
      clean: true,
      connectTimeout: 8000,
      reconnectPeriod: 0,
      keepalive: 30,
    });
    link.client = client;

    client.on('connect', () => {
      link.connected = true;
      link.failures = 0;
      this.subscribeAll(link);
      this.publishPresence(true, null, link);
      this.emitStatus();
    });
    client.on('message', (topic, payload) => this.handleMessage(topic, payload, link));
    client.on('error', () => {});
    client.on('close', () => {
      if (link.client !== client) {
        return;
      }
      const wasConnected = link.connected;
      link.client = null;
      link.connected = false;
      if (wasConnected) {
        this.emitStatus();
      }
      if (this.stopped) {
        return;
      }
      link.failures = wasConnected ? 0 : link.failures + 1;
      const delay = wasConnected
        ? RECONNECT_DELAY_MS
        : Math.min(MAX_RETRY_DELAY_MS, RETRY_DELAY_MS * 2 ** (link.failures - 1));
      link.retryTimer = setTimeout(() => this.connect(link), delay);
    });
  }

  openLinks() {
    return this.links.filter((link) => link.connected && link.client);
  }

  spaceIds() {
    return this.spaces.list().map((space) => space.id);
  }

  subscribeAll(link) {
    const topics = this.spaceIds().map((id) => TOPIC_PREFIX + id);
    if (topics.length > 0) {
      link.client.subscribe(topics, { qos: 1 });
    }
  }

  addSpace(spaceId) {
    const open = this.openLinks();
    for (const link of open) {
      link.client.subscribe(TOPIC_PREFIX + spaceId, { qos: 1 });
    }
    if (open.length > 0) {
      this.publishPresence(true, spaceId);
    }
  }

  addContact(contactId) {
    this.addSpace(contactId);
  }

  removeSpace(spaceId) {
    this.publish(spaceId, 'presence', { online: false, name: this.profileName });
    for (const link of this.openLinks()) {
      link.client.unsubscribe(TOPIC_PREFIX + spaceId);
    }
    this.presence.delete(spaceId);
    this.routes.delete(spaceId);
  }

  removeContact(contactId) {
    this.removeSpace(contactId);
  }

  // onAcked recebe o erro, ou nada quando algum servidor confirmou a mensagem.
  publish(spaceId, type, body = {}, onAcked = undefined) {
    return this.send(spaceId, type, body, onAcked, null);
  }

  send(spaceId, type, body, onAcked, only) {
    const keys = this.spaces.keysFor(spaceId);
    const open = this.openLinks().filter((link) => !only || link === only);
    if (!keys || open.length === 0) {
      return false;
    }
    const payload = sealMessage(keys.key, {
      ...body, type, from: this.instanceId, at: Date.now(), mid: crypto.randomUUID(),
    });
    const targets = only ? open : this.routeFor(spaceId, payload.length, open);
    let settled = false;
    let failures = 0;
    const done = onAcked
      ? (error) => {
        if (settled) {
          return;
        }
        if (!error) {
          settled = true;
          onAcked();
        } else if (++failures === targets.length) {
          settled = true;
          onAcked(error);
        }
      }
      : undefined;
    for (const link of targets) {
      link.client.publish(TOPIC_PREFIX + spaceId, payload, { qos: 1 }, done);
    }
    return true;
  }

  // Mensagem pequena vai por todos os servidores. Mensagem grande (pedaco de arquivo, imagem,
  // miniatura) vai so pelos servidores onde estao as pessoas online, para nao pagar o envio tres vezes.
  routeFor(spaceId, size, open) {
    if (size <= WIDE_PAYLOAD_BYTES || open.length < 2) {
      return open;
    }
    const now = Date.now();
    const routes = this.routes.get(spaceId);
    let left = this.peers(spaceId).map((peer) => {
      const via = routes ? routes.get(peer.id) : null;
      return new Set(via ? [...via].filter(([, at]) => now - at < PRESENCE_TTL_MS).map(([url]) => url) : []);
    });
    if (left.length === 0) {
      return [open[0]];
    }
    const chosen = [];
    while (left.length > 0) {
      let best = null;
      let bestCount = 0;
      for (const link of open) {
        const count = left.filter((urls) => urls.has(link.url)).length;
        if (count > bestCount) {
          best = link;
          bestCount = count;
        }
      }
      if (!best) {
        return open;
      }
      chosen.push(best);
      left = left.filter((urls) => !urls.has(best.url));
    }
    return chosen;
  }

  presencePayload(spaceId) {
    const extra = this.presenceFor ? this.presenceFor(spaceId) : null;
    return { name: this.profileName, ...extra, online: true };
  }

  publishPresence(online, spaceId = null, only = null) {
    const ids = spaceId ? [spaceId] : this.spaceIds();
    for (const id of ids) {
      const body = online ? this.presencePayload(id) : { online: false, name: this.profileName };
      this.send(id, 'presence', body, undefined, only);
    }
  }

  noteRoute(spaceId, peerId, link) {
    let routes = this.routes.get(spaceId);
    if (!routes) {
      routes = new Map();
      this.routes.set(spaceId, routes);
    }
    let via = routes.get(peerId);
    if (!via) {
      via = new Map();
      routes.set(peerId, via);
    }
    via.set(link.url, Date.now());
  }

  // Mensagens de versoes antigas nao tem id, mas elas so usam um servidor e nunca chegam repetidas.
  firstSight(message) {
    if (typeof message.mid !== 'string') {
      return true;
    }
    if (this.seen.has(message.mid)) {
      return false;
    }
    this.seen.set(message.mid, Date.now());
    if (this.seen.size > MAX_SEEN) {
      this.seen.delete(this.seen.keys().next().value);
    }
    return true;
  }

  expireSeen() {
    const now = Date.now();
    for (const [mid, at] of this.seen) {
      if (now - at < SEEN_TTL_MS) {
        return;
      }
      this.seen.delete(mid);
    }
  }

  handleMessage(topic, payload, link) {
    if (!topic.startsWith(TOPIC_PREFIX)) {
      return;
    }
    const spaceId = topic.slice(TOPIC_PREFIX.length);
    const keys = this.spaces.keysFor(spaceId);
    if (!keys) {
      return;
    }
    const message = openMessage(keys.key, payload);
    if (!message || typeof message.from !== 'string' || message.from === this.instanceId) {
      return;
    }
    this.noteRoute(spaceId, message.from, link);
    if (!this.firstSight(message)) {
      return;
    }

    if (message.type !== 'presence') {
      this.emit('message', { contactId: spaceId, spaceId, message });
      return;
    }

    const peers = this.peerMap(spaceId);
    const previous = peers.get(message.from);
    const known = Boolean(previous && Date.now() - previous.seenAt < PRESENCE_TTL_MS);
    const wasOnline = this.isOnline(spaceId);
    let listChanged = false;

    if (message.online) {
      const info = describePeer(message);
      listChanged = !known || JSON.stringify(previous.info) !== JSON.stringify(info);
      peers.set(message.from, { seenAt: Date.now(), info });
      if (info.name) {
        this.emit('peer-name', { contactId: spaceId, spaceId, name: info.name });
      }
      this.emit('peer-presence', { spaceId, peerId: message.from, message });
      if (!known) {
        this.publishPresence(true, spaceId);
      }
    } else if (peers.delete(message.from)) {
      listChanged = true;
    }

    if (listChanged) {
      this.emit('peers', { spaceId, peers: this.peers(spaceId) });
    }
    if (wasOnline !== this.isOnline(spaceId)) {
      this.emit('presence', { contactId: spaceId, spaceId, online: this.isOnline(spaceId) });
    }
  }

  peerMap(spaceId) {
    let peers = this.presence.get(spaceId);
    if (!peers) {
      peers = new Map();
      this.presence.set(spaceId, peers);
    }
    return peers;
  }

  peers(spaceId) {
    const peers = this.presence.get(spaceId);
    if (!peers) {
      return [];
    }
    const now = Date.now();
    return [...peers]
      .filter(([, entry]) => now - entry.seenAt < PRESENCE_TTL_MS)
      .map(([id, entry]) => ({ id, ...entry.info }));
  }

  snapshot() {
    return Object.fromEntries(this.spaceIds().map((id) => [id, this.peers(id)]));
  }

  isOnline(spaceId) {
    return this.peers(spaceId).length > 0;
  }

  expirePresence() {
    const now = Date.now();
    for (const [spaceId, peers] of this.presence) {
      let changed = false;
      for (const [peerId, entry] of peers) {
        if (now - entry.seenAt >= PRESENCE_TTL_MS) {
          peers.delete(peerId);
          changed = true;
        }
      }
      const routes = this.routes.get(spaceId);
      if (routes) {
        for (const peerId of routes.keys()) {
          if (!peers.has(peerId)) {
            routes.delete(peerId);
          }
        }
      }
      if (changed) {
        this.emit('peers', { spaceId, peers: this.peers(spaceId) });
        if (peers.size === 0) {
          this.emit('presence', { contactId: spaceId, spaceId, online: false });
        }
      }
    }
  }

  status() {
    const open = this.openLinks();
    return {
      connected: open.length > 0,
      broker: open.length > 0 ? open[0].url : null,
      brokers: open.map((link) => link.url),
    };
  }

  emitStatus() {
    this.emit('status', this.status());
  }

  stop() {
    this.stopped = true;
    clearInterval(this.presenceTimer);
    const goodbyes = this.spaceIds().map((id) => {
      const keys = this.spaces.keysFor(id);
      return keys ? [id, sealMessage(keys.key, {
        type: 'presence', online: false, from: this.instanceId, at: Date.now(), mid: crypto.randomUUID(),
      })] : null;
    }).filter(Boolean);
    const endings = this.links.map((link) => {
      clearTimeout(link.retryTimer);
      const { client } = link;
      link.client = null;
      if (!client) {
        return Promise.resolve();
      }
      if (link.connected) {
        for (const [id, payload] of goodbyes) {
          client.publish(TOPIC_PREFIX + id, payload, { qos: 0 });
        }
      }
      link.connected = false;
      return new Promise((resolve) => client.end(false, {}, () => resolve()));
    });
    return Promise.all(endings);
  }
}

module.exports = { DEFAULT_BROKERS, PEER_ID_PATTERN, Signaling, TOPIC_PREFIX };
