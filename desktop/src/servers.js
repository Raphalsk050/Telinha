'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { cleanName, contactKeys } = require('./contacts');

const INVITE_PREFIX = 'TELINHA-GRUPO.';
const INVITE_PATTERN = /TELINHA-GRUPO\.([A-Za-z0-9_-]+)/;
const CHANNEL_ID_PATTERN = /^[a-f0-9]{16}$/;
const MEMBER_ID_PATTERN = /^[a-f0-9]{32}$/;
const CHANNEL_KINDS = new Set(['text', 'voice']);
const MAX_CHANNELS = 50;
const MAX_MEMBERS = 500;
const MAX_CHANNEL_NAME = 32;

function newChannelId() {
  return crypto.randomBytes(8).toString('hex');
}

function cleanChannelName(name, kind, fallback) {
  const collapsed = String(name ?? '').trim().replace(/\s+/g, kind === 'text' ? '-' : ' ');
  const clean = (kind === 'text' ? collapsed.toLowerCase() : collapsed).slice(0, MAX_CHANNEL_NAME);
  return clean || fallback;
}

function sanitizeChannels(channels) {
  if (!Array.isArray(channels)) {
    return null;
  }
  const seen = new Set();
  const result = [];
  for (const channel of channels.slice(0, MAX_CHANNELS)) {
    if (!channel || !CHANNEL_ID_PATTERN.test(String(channel.id)) || seen.has(channel.id)
      || !CHANNEL_KINDS.has(channel.kind)) {
      continue;
    }
    seen.add(channel.id);
    result.push({
      id: channel.id,
      kind: channel.kind,
      name: cleanChannelName(channel.name, channel.kind, channel.kind === 'text' ? 'canal' : 'Sala'),
    });
  }
  return result;
}

function nextVersion(current) {
  return Math.max(Date.now(), (Number(current) || 0) + 1);
}

function encodeInvite(secret, name) {
  const body = Buffer.from(JSON.stringify({ v: 1, s: secret.toString('base64'), n: name }), 'utf8');
  return `${INVITE_PREFIX}${body.toString('base64url')}`;
}

function decodeInvite(text) {
  const match = String(text ?? '').match(INVITE_PATTERN);
  if (!match) {
    return null;
  }
  try {
    const data = JSON.parse(Buffer.from(match[1], 'base64url').toString('utf8'));
    const secret = Buffer.from(String(data.s), 'base64');
    if (data.v !== 1 || secret.length !== 32) {
      return null;
    }
    return { secret, name: cleanName(data.n, 'Servidor') };
  } catch {
    return null;
  }
}

function publicView(server) {
  return {
    id: server.id,
    name: server.name,
    createdAt: server.createdAt,
    channels: server.channels.map((channel) => ({ ...channel })),
    members: Object.entries(server.members).map(([memberId, member]) => ({
      memberId,
      name: member.name,
      lastSeenAt: member.lastSeenAt,
    })),
  };
}

class ServerStore {
  constructor(filePath, { encrypt = (text) => text, decrypt = (text) => text } = {}) {
    this.filePath = filePath;
    this.encrypt = encrypt;
    this.decrypt = decrypt;
    this.servers = [];
    this.keyCache = new Map();
  }

  load() {
    try {
      const data = JSON.parse(fs.readFileSync(this.filePath, 'utf8'));
      this.servers = Array.isArray(data.servers)
        ? data.servers
          .filter((server) => server && server.id && server.secret)
          .map((server) => ({
            id: server.id,
            name: cleanName(server.name, 'Servidor'),
            nameVersion: Number(server.nameVersion) || 0,
            secret: server.secret,
            createdAt: Number(server.createdAt) || Date.now(),
            channels: sanitizeChannels(server.channels) ?? [],
            channelsVersion: Number(server.channelsVersion) || 0,
            members: server.members && typeof server.members === 'object' ? server.members : {},
          }))
        : [];
    } catch {
      this.servers = [];
    }
    this.keyCache.clear();
    return this;
  }

  save() {
    fs.mkdirSync(path.dirname(this.filePath), { recursive: true });
    const temporary = `${this.filePath}.tmp`;
    fs.writeFileSync(temporary, JSON.stringify({ version: 1, servers: this.servers }, null, 2));
    fs.renameSync(temporary, this.filePath);
  }

  list() {
    return this.servers.map(publicView);
  }

  get(id) {
    return this.servers.find((server) => server.id === id) ?? null;
  }

  view(id) {
    const server = this.get(id);
    return server ? publicView(server) : null;
  }

  secretFor(server) {
    return Buffer.from(this.decrypt(server.secret), 'base64');
  }

  keysFor(id) {
    if (this.keyCache.has(id)) {
      return this.keyCache.get(id);
    }
    const server = this.get(id);
    if (!server) {
      return null;
    }
    try {
      const keys = contactKeys(this.secretFor(server));
      this.keyCache.set(id, keys);
      return keys;
    } catch {
      return null;
    }
  }

  meta(id) {
    const server = this.get(id);
    return server ? {
      name: server.name,
      nameVersion: server.nameVersion,
      channels: server.channels,
      channelsVersion: server.channelsVersion,
    } : null;
  }

  addServer(secret, name, nameVersion, channels, channelsVersion, self) {
    const { id } = contactKeys(secret);
    const now = Date.now();
    const server = {
      id,
      name: cleanName(name, 'Servidor'),
      nameVersion,
      secret: this.encrypt(secret.toString('base64')),
      createdAt: now,
      channels,
      channelsVersion,
      members: {},
    };
    if (self && MEMBER_ID_PATTERN.test(String(self.memberId))) {
      server.members[self.memberId] = { name: cleanName(self.name, 'Você'), lastSeenAt: now };
    }
    this.servers.push(server);
    this.save();
    return server;
  }

  create(name, self) {
    const now = Date.now();
    const channels = [
      { id: newChannelId(), kind: 'text', name: 'geral' },
      { id: newChannelId(), kind: 'voice', name: 'Geral' },
    ];
    const server = this.addServer(crypto.randomBytes(32), cleanName(name, 'Meu servidor'), now, channels, now, self);
    return publicView(server);
  }

  join(code, self) {
    const invite = decodeInvite(code);
    if (!invite) {
      return null;
    }
    const { id } = contactKeys(invite.secret);
    const existing = this.get(id);
    if (existing) {
      return { server: publicView(existing), created: false };
    }
    const server = this.addServer(invite.secret, invite.name, 0, [], 0, self);
    return { server: publicView(server), created: true };
  }

  inviteCode(id) {
    const server = this.get(id);
    return server ? encodeInvite(this.secretFor(server), server.name) : null;
  }

  rename(id, name) {
    const server = this.get(id);
    const clean = cleanName(name, '');
    if (!server || !clean || clean === server.name) {
      return false;
    }
    server.name = clean;
    server.nameVersion = nextVersion(server.nameVersion);
    this.save();
    return true;
  }

  leave(id) {
    const server = this.get(id);
    if (!server) {
      return null;
    }
    this.servers = this.servers.filter((item) => item.id !== id);
    this.keyCache.delete(id);
    this.save();
    return publicView(server);
  }

  applyMeta(id, meta) {
    const server = this.get(id);
    const result = { changed: false, mineNewer: false };
    if (!server || !meta || typeof meta !== 'object') {
      return result;
    }

    const channelsVersion = Number(meta.channelsVersion) || 0;
    if (channelsVersion > server.channelsVersion) {
      const channels = sanitizeChannels(meta.channels);
      if (channels) {
        server.channels = channels;
        server.channelsVersion = channelsVersion;
        result.changed = true;
      }
    } else if (channelsVersion < server.channelsVersion) {
      result.mineNewer = true;
    }

    const nameVersion = Number(meta.nameVersion) || 0;
    if (nameVersion > server.nameVersion && typeof meta.name === 'string') {
      server.name = cleanName(meta.name, server.name);
      server.nameVersion = nameVersion;
      result.changed = true;
    } else if (nameVersion < server.nameVersion) {
      result.mineNewer = true;
    }

    if (result.changed) {
      this.save();
    }
    return result;
  }

  touchMember(id, memberId, name) {
    const server = this.get(id);
    if (!server || !MEMBER_ID_PATTERN.test(String(memberId))) {
      return false;
    }
    const clean = cleanName(name, 'Alguém');
    const existing = server.members[memberId];
    const now = Date.now();
    if (existing && existing.name === clean) {
      existing.lastSeenAt = now;
      return false;
    }
    if (!existing && Object.keys(server.members).length >= MAX_MEMBERS) {
      return false;
    }
    server.members[memberId] = { name: clean, lastSeenAt: now };
    this.save();
    return true;
  }

  addChannel(id, kind, name) {
    const server = this.get(id);
    if (!server || !CHANNEL_KINDS.has(kind) || server.channels.length >= MAX_CHANNELS) {
      return null;
    }
    const channel = {
      id: newChannelId(),
      kind,
      name: cleanChannelName(name, kind, kind === 'text' ? 'novo-canal' : 'Nova sala'),
    };
    server.channels.push(channel);
    server.channelsVersion = nextVersion(server.channelsVersion);
    this.save();
    return { ...channel };
  }

  renameChannel(id, channelId, name) {
    const server = this.get(id);
    const channel = server ? server.channels.find((item) => item.id === channelId) : null;
    if (!channel) {
      return false;
    }
    const clean = cleanChannelName(name, channel.kind, channel.name);
    if (clean === channel.name) {
      return false;
    }
    channel.name = clean;
    server.channelsVersion = nextVersion(server.channelsVersion);
    this.save();
    return true;
  }

  removeChannel(id, channelId) {
    const server = this.get(id);
    if (!server || !server.channels.some((item) => item.id === channelId)) {
      return false;
    }
    server.channels = server.channels.filter((item) => item.id !== channelId);
    server.channelsVersion = nextVersion(server.channelsVersion);
    this.save();
    return true;
  }
}

module.exports = { INVITE_PREFIX, ServerStore, decodeInvite, encodeInvite };
