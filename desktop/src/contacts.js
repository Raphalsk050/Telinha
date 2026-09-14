'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { normalizeCode } = require('./telinha-process');

const ENVELOPE_VERSION = 1;
const IV_BYTES = 12;
const TAG_BYTES = 16;
const MAX_CLOCK_SKEW_MS = 5 * 60 * 1000;
const MAX_NAME_LENGTH = 40;

function deriveContactSecret(inviteCode, answerCode) {
  return crypto.createHash('sha256')
    .update('telinha-contact-v1\n')
    .update(normalizeCode(inviteCode))
    .update('\n')
    .update(normalizeCode(answerCode))
    .digest();
}

function contactKeys(secret) {
  const id = crypto.createHmac('sha256', secret).update('telinha-contact-id').digest('hex').slice(0, 32);
  const key = Buffer.from(crypto.hkdfSync('sha256', secret, Buffer.alloc(0), 'telinha-signaling-key', 32));
  return { id, key };
}

function sealMessage(key, message) {
  const iv = crypto.randomBytes(IV_BYTES);
  const cipher = crypto.createCipheriv('aes-256-gcm', key, iv);
  const body = Buffer.concat([cipher.update(JSON.stringify(message), 'utf8'), cipher.final()]);
  return Buffer.concat([Buffer.from([ENVELOPE_VERSION]), iv, cipher.getAuthTag(), body]).toString('base64');
}

function openMessage(key, payload, now = Date.now()) {
  const data = Buffer.from(String(payload), 'base64');
  if (data.length <= 1 + IV_BYTES + TAG_BYTES || data[0] !== ENVELOPE_VERSION) {
    return null;
  }
  try {
    const decipher = crypto.createDecipheriv('aes-256-gcm', key, data.subarray(1, 1 + IV_BYTES));
    decipher.setAuthTag(data.subarray(1 + IV_BYTES, 1 + IV_BYTES + TAG_BYTES));
    const text = Buffer.concat([
      decipher.update(data.subarray(1 + IV_BYTES + TAG_BYTES)),
      decipher.final(),
    ]).toString('utf8');
    const message = JSON.parse(text);
    if (!message || typeof message.type !== 'string' || !Number.isFinite(message.at)) {
      return null;
    }
    return Math.abs(now - message.at) > MAX_CLOCK_SKEW_MS ? null : message;
  } catch {
    return null;
  }
}

function cleanName(name, fallback = 'Contato') {
  const clean = String(name ?? '').replace(/\s+/g, ' ').trim().slice(0, MAX_NAME_LENGTH);
  return clean || fallback;
}

function publicView(contact) {
  return {
    id: contact.id,
    name: contact.name,
    createdAt: contact.createdAt,
    lastConnectedAt: contact.lastConnectedAt,
  };
}

class ContactStore {
  constructor(filePath, { encrypt = (text) => text, decrypt = (text) => text } = {}) {
    this.filePath = filePath;
    this.encrypt = encrypt;
    this.decrypt = decrypt;
    this.contacts = [];
    this.keyCache = new Map();
  }

  load() {
    try {
      const data = JSON.parse(fs.readFileSync(this.filePath, 'utf8'));
      this.contacts = Array.isArray(data.contacts)
        ? data.contacts.filter((contact) => contact && contact.id && contact.secret)
        : [];
    } catch {
      this.contacts = [];
    }
    this.keyCache.clear();
    return this;
  }

  save() {
    fs.mkdirSync(path.dirname(this.filePath), { recursive: true });
    const temporary = `${this.filePath}.tmp`;
    fs.writeFileSync(temporary, JSON.stringify({ version: 1, contacts: this.contacts }, null, 2));
    fs.renameSync(temporary, this.filePath);
  }

  list() {
    return this.contacts.map(publicView);
  }

  get(id) {
    return this.contacts.find((contact) => contact.id === id) ?? null;
  }

  keysFor(id) {
    if (this.keyCache.has(id)) {
      return this.keyCache.get(id);
    }
    const contact = this.get(id);
    if (!contact) {
      return null;
    }
    try {
      const keys = contactKeys(Buffer.from(this.decrypt(contact.secret), 'base64'));
      this.keyCache.set(id, keys);
      return keys;
    } catch {
      return null;
    }
  }

  saveFromCodes(inviteCode, answerCode, name) {
    const secret = deriveContactSecret(inviteCode, answerCode);
    const { id } = contactKeys(secret);
    const now = Date.now();

    let contact = this.get(id);
    const created = !contact;
    if (created) {
      contact = {
        id,
        name: cleanName(name),
        nameCustomized: false,
        createdAt: now,
        lastConnectedAt: now,
        secret: this.encrypt(secret.toString('base64')),
      };
      this.contacts.push(contact);
    } else {
      contact.lastConnectedAt = now;
    }
    this.save();
    return { contact: publicView(contact), created };
  }

  rename(id, name) {
    const contact = this.get(id);
    if (!contact) {
      return false;
    }
    contact.name = cleanName(name, contact.name);
    contact.nameCustomized = true;
    this.save();
    return true;
  }

  adoptRemoteName(id, name) {
    const contact = this.get(id);
    const clean = cleanName(name, '');
    if (!contact || contact.nameCustomized || !clean || clean === contact.name) {
      return false;
    }
    contact.name = clean;
    this.save();
    return true;
  }

  touch(id) {
    const contact = this.get(id);
    if (!contact) {
      return false;
    }
    contact.lastConnectedAt = Date.now();
    this.save();
    return true;
  }

  remove(id) {
    const before = this.contacts.length;
    this.contacts = this.contacts.filter((contact) => contact.id !== id);
    this.keyCache.delete(id);
    if (this.contacts.length === before) {
      return false;
    }
    this.save();
    return true;
  }
}

module.exports = {
  ContactStore,
  cleanName,
  contactKeys,
  deriveContactSecret,
  openMessage,
  sealMessage,
};
