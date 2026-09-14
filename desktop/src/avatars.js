'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const MAX_AVATAR_BYTES = 200 * 1024;
const MEMBER_ID_PATTERN = /^[a-f0-9]{32}$/;
const SIGNATURES = new Map([
  ['image/webp', (data) => data.length > 12 && data.toString('ascii', 0, 4) === 'RIFF'
    && data.toString('ascii', 8, 12) === 'WEBP'],
  ['image/png', (data) => data.length > 8 && data.readUInt32BE(0) === 0x89504e47],
  ['image/jpeg', (data) => data.length > 3 && data[0] === 0xff && data[1] === 0xd8 && data[2] === 0xff],
]);

function decodeAvatar(mime, base64) {
  if (typeof mime !== 'string' || !SIGNATURES.has(mime) || typeof base64 !== 'string') {
    return null;
  }
  const data = Buffer.from(base64, 'base64');
  if (data.length === 0 || data.length > MAX_AVATAR_BYTES || !SIGNATURES.get(mime)(data)) {
    return null;
  }
  const hash = crypto.createHash('sha256').update(data).digest('hex').slice(0, 32);
  return { mime, data, hash };
}

function dataUrl(mime, data) {
  return `data:${mime};base64,${data.toString('base64')}`;
}

class AvatarStore {
  constructor(directory) {
    this.directory = directory;
    this.indexFile = path.join(directory, 'index.json');
    this.members = {};
    this.contacts = {};
  }

  load() {
    try {
      const data = JSON.parse(fs.readFileSync(this.indexFile, 'utf8'));
      for (const [memberId, entry] of Object.entries(data.members || {})) {
        if (MEMBER_ID_PATTERN.test(memberId) && entry && SIGNATURES.has(entry.mime) && typeof entry.hash === 'string') {
          this.members[memberId] = { mime: entry.mime, hash: entry.hash };
        }
      }
      for (const [contactId, memberId] of Object.entries(data.contacts || {})) {
        if (MEMBER_ID_PATTERN.test(String(memberId))) {
          this.contacts[contactId] = memberId;
        }
      }
    } catch {
      this.members = {};
      this.contacts = {};
    }
    return this;
  }

  save() {
    fs.mkdirSync(this.directory, { recursive: true });
    const temporary = `${this.indexFile}.tmp`;
    fs.writeFileSync(temporary, JSON.stringify({ version: 1, members: this.members, contacts: this.contacts }, null, 2));
    fs.renameSync(temporary, this.indexFile);
  }

  fileFor(memberId) {
    return path.join(this.directory, `${memberId}.bin`);
  }

  hashFor(memberId) {
    const entry = this.members[memberId];
    return entry ? entry.hash : '';
  }

  put(memberId, avatar) {
    if (!MEMBER_ID_PATTERN.test(String(memberId))) {
      return false;
    }
    fs.mkdirSync(this.directory, { recursive: true });
    fs.writeFileSync(this.fileFor(memberId), avatar.data);
    this.members[memberId] = { mime: avatar.mime, hash: avatar.hash };
    this.save();
    return true;
  }

  remove(memberId) {
    if (!this.members[memberId]) {
      return false;
    }
    delete this.members[memberId];
    fs.rmSync(this.fileFor(memberId), { force: true });
    this.save();
    return true;
  }

  url(memberId) {
    const entry = this.members[memberId];
    if (!entry) {
      return null;
    }
    try {
      return dataUrl(entry.mime, fs.readFileSync(this.fileFor(memberId)));
    } catch {
      return null;
    }
  }

  linkContact(contactId, memberId) {
    if (!MEMBER_ID_PATTERN.test(String(memberId)) || this.contacts[contactId] === memberId) {
      return false;
    }
    this.contacts[contactId] = memberId;
    this.save();
    return true;
  }

  snapshot() {
    const members = {};
    for (const memberId of Object.keys(this.members)) {
      const url = this.url(memberId);
      if (url) {
        members[memberId] = url;
      }
    }
    return { members, contacts: { ...this.contacts } };
  }
}

module.exports = { AvatarStore, MAX_AVATAR_BYTES, dataUrl, decodeAvatar };
