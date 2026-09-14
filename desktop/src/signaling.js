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
const RECONNECT_DELAY_MS = 1000;
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

class Signaling extends EventEmitter {
  constructor({
    store, spaces = store, profileName, presenceFor = null, brokers = DEFAULT_BROKERS, connect = defaultConnect,
  }) {
    super();
    this.spaces = spaces;
    this.profileName = profileName;
    this.presenceFor = presenceFor;
    this.brokers = brokers;
    this.connectClient = connect;
    this.instanceId = crypto.randomUUID();
    this.client = null;
    this.connected = false;
    this.broker = null;
    this.brokerIndex = 0;
    this.stopped = true;
    this.presence = new Map();
    this.presenceTimer = null;
    this.retryTimer = null;
  }

  start() {
    this.stopped = false;
    this.connectNext();
    this.presenceTimer = setInterval(() => {
      this.publishPresence(true);
      this.expirePresence();
    }, PRESENCE_INTERVAL_MS);
  }

  connectNext() {
    if (this.stopped) {
      return;
    }
    const url = this.brokers[this.brokerIndex % this.brokers.length];
    const client = this.connectClient(url, {
      clientId: `telinha-${this.instanceId.slice(0, 8)}-${Date.now().toString(36)}`,
      clean: true,
      connectTimeout: 8000,
      reconnectPeriod: 0,
      keepalive: 30,
    });
    this.client = client;

    client.on('connect', () => {
      this.connected = true;
      this.broker = url;
      this.subscribeAll();
      this.publishPresence(true);
      this.emitStatus();
    });
    client.on('message', (topic, payload) => this.handleMessage(topic, payload));
    client.on('error', () => {});
    client.on('close', () => {
      if (this.client !== client) {
        return;
      }
      const wasConnected = this.connected;
      this.client = null;
      this.connected = false;
      this.broker = null;
      this.emitStatus();
      if (this.stopped) {
        return;
      }
      if (!wasConnected) {
        this.brokerIndex += 1;
      }
      this.retryTimer = setTimeout(() => this.connectNext(),
        wasConnected ? RECONNECT_DELAY_MS : RETRY_DELAY_MS);
    });
  }

  spaceIds() {
    return this.spaces.list().map((space) => space.id);
  }

  subscribeAll() {
    const topics = this.spaceIds().map((id) => TOPIC_PREFIX + id);
    if (this.client && topics.length > 0) {
      this.client.subscribe(topics, { qos: 1 });
    }
  }

  addSpace(spaceId) {
    if (this.client && this.connected) {
      this.client.subscribe(TOPIC_PREFIX + spaceId, { qos: 1 });
      this.publishPresence(true, spaceId);
    }
  }

  addContact(contactId) {
    this.addSpace(contactId);
  }

  removeSpace(spaceId) {
    if (this.client && this.connected) {
      this.publish(spaceId, 'presence', { online: false, name: this.profileName });
      this.client.unsubscribe(TOPIC_PREFIX + spaceId);
    }
    this.presence.delete(spaceId);
  }

  removeContact(contactId) {
    this.removeSpace(contactId);
  }

  publish(spaceId, type, body = {}) {
    const keys = this.spaces.keysFor(spaceId);
    if (!keys || !this.client || !this.connected) {
      return false;
    }
    const payload = sealMessage(keys.key, { ...body, type, from: this.instanceId, at: Date.now() });
    this.client.publish(TOPIC_PREFIX + spaceId, payload, { qos: 1 });
    return true;
  }

  presencePayload(spaceId) {
    const extra = this.presenceFor ? this.presenceFor(spaceId) : null;
    return { name: this.profileName, ...extra, online: true };
  }

  publishPresence(online, spaceId = null) {
    const ids = spaceId ? [spaceId] : this.spaceIds();
    for (const id of ids) {
      this.publish(id, 'presence', online ? this.presencePayload(id) : { online: false, name: this.profileName });
    }
  }

  handleMessage(topic, payload) {
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
      if (changed) {
        this.emit('peers', { spaceId, peers: this.peers(spaceId) });
        if (peers.size === 0) {
          this.emit('presence', { contactId: spaceId, spaceId, online: false });
        }
      }
    }
  }

  status() {
    return { connected: this.connected, broker: this.broker };
  }

  emitStatus() {
    this.emit('status', this.status());
  }

  stop() {
    this.stopped = true;
    clearInterval(this.presenceTimer);
    clearTimeout(this.retryTimer);
    const client = this.client;
    this.client = null;
    if (!client) {
      return Promise.resolve();
    }
    if (this.connected) {
      for (const id of this.spaceIds()) {
        const keys = this.spaces.keysFor(id);
        if (keys) {
          const payload = sealMessage(keys.key, {
            type: 'presence', online: false, from: this.instanceId, at: Date.now(),
          });
          client.publish(TOPIC_PREFIX + id, payload, { qos: 0 });
        }
      }
    }
    this.connected = false;
    return new Promise((resolve) => client.end(false, {}, () => resolve()));
  }
}

module.exports = { DEFAULT_BROKERS, PEER_ID_PATTERN, Signaling, TOPIC_PREFIX };
