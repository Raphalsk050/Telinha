'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const MAX_MESSAGES = 500;
const MAX_TEXT_LENGTH = 2000;
const MAX_IMAGE_BYTES = 400 * 1024;
const MAX_IMAGE_SIDE = 8192;
const BLOB_ID_PATTERN = /^[A-Za-z0-9_-]{8,64}$/;
const IMAGE_SIGNATURES = new Map([
  ['image/webp', (data) => data.length > 12 && data.toString('ascii', 0, 4) === 'RIFF'
    && data.toString('ascii', 8, 12) === 'WEBP'],
  ['image/jpeg', (data) => data.length > 3 && data[0] === 0xff && data[1] === 0xd8 && data[2] === 0xff],
  ['image/png', (data) => data.length > 8 && data.readUInt32BE(0) === 0x89504e47],
  ['image/gif', (data) => data.length > 6 && data.toString('ascii', 0, 4) === 'GIF8'],
]);

function cleanText(text) {
  return String(text ?? '').replace(/\r\n/g, '\n').trim().slice(0, MAX_TEXT_LENGTH);
}

function safeId(conversationId) {
  return String(conversationId).replace(/[^A-Za-z0-9_-]/g, '');
}

function decodeImage(image) {
  if (!image || typeof image !== 'object' || typeof image.data !== 'string' || !IMAGE_SIGNATURES.has(image.mime)) {
    return null;
  }
  const data = Buffer.from(image.data, 'base64');
  if (data.length === 0 || data.length > MAX_IMAGE_BYTES || !IMAGE_SIGNATURES.get(image.mime)(data)) {
    return null;
  }
  const width = Math.floor(Number(image.width));
  const height = Math.floor(Number(image.height));
  if (!(width > 0 && width <= MAX_IMAGE_SIDE && height > 0 && height <= MAX_IMAGE_SIDE)) {
    return null;
  }
  return { mime: image.mime, width, height, data };
}

class ChatStore {
  constructor(directory, { encrypt = (text) => text, decrypt = (text) => text } = {}) {
    this.directory = directory;
    this.encrypt = encrypt;
    this.decrypt = decrypt;
    this.cache = new Map();
  }

  fileFor(conversationId) {
    return path.join(this.directory, `${safeId(conversationId)}.json`);
  }

  blobDirectory() {
    return path.join(this.directory, 'images');
  }

  blobFile(conversationId, blobId) {
    return path.join(this.blobDirectory(), `${safeId(conversationId)}_${safeId(blobId)}.bin`);
  }

  history(conversationId) {
    if (!this.cache.has(conversationId)) {
      let messages = [];
      try {
        const stored = fs.readFileSync(this.fileFor(conversationId), 'utf8');
        const data = JSON.parse(this.decrypt(stored));
        if (Array.isArray(data.messages)) {
          messages = data.messages;
        }
      } catch {
        messages = [];
      }
      this.cache.set(conversationId, messages);
    }
    return this.cache.get(conversationId);
  }

  createOutgoing(author, text, { allowEmpty = false } = {}) {
    const clean = cleanText(text);
    if (!clean && !allowEmpty) {
      return null;
    }
    return { id: crypto.randomUUID(), author, mine: true, text: clean, sentAt: Date.now() };
  }

  saveBlob(conversationId, blobId, data) {
    fs.mkdirSync(this.blobDirectory(), { recursive: true });
    const file = this.blobFile(conversationId, blobId);
    const temporary = `${file}.tmp`;
    fs.writeFileSync(temporary, this.encrypt(data.toString('base64')));
    fs.renameSync(temporary, file);
  }

  loadBlob(conversationId, blobId) {
    try {
      const stored = fs.readFileSync(this.blobFile(conversationId, blobId), 'utf8');
      return Buffer.from(this.decrypt(stored), 'base64');
    } catch {
      return null;
    }
  }

  removeBlob(conversationId, blobId) {
    fs.rmSync(this.blobFile(conversationId, blobId), { force: true });
  }

  append(conversationId, message) {
    const messages = this.history(conversationId);
    if (messages.some((existing) => existing.id === message.id)) {
      return null;
    }
    messages.push(message);
    messages.sort((a, b) => a.sentAt - b.sentAt);
    if (messages.length > MAX_MESSAGES) {
      const removed = messages.splice(0, messages.length - MAX_MESSAGES);
      for (const old of removed) {
        for (const blob of [old.image, old.file]) {
          if (blob && blob.id) {
            this.removeBlob(conversationId, blob.id);
          }
        }
      }
    }
    this.save(conversationId);
    return message;
  }

  save(conversationId) {
    fs.mkdirSync(this.directory, { recursive: true });
    const file = this.fileFor(conversationId);
    const temporary = `${file}.tmp`;
    const payload = JSON.stringify({ version: 1, messages: this.history(conversationId) });
    fs.writeFileSync(temporary, this.encrypt(payload));
    fs.renameSync(temporary, file);
  }

  forget(conversationId) {
    this.cache.delete(conversationId);
    fs.rmSync(this.fileFor(conversationId), { force: true });
    const prefix = `${safeId(conversationId)}_`;
    try {
      for (const name of fs.readdirSync(this.blobDirectory())) {
        if (name.startsWith(prefix)) {
          fs.rmSync(path.join(this.blobDirectory(), name), { force: true });
        }
      }
    } catch {
      // ainda nao existe pasta de anexos
    }
  }
}

module.exports = {
  BLOB_ID_PATTERN, ChatStore, MAX_IMAGE_BYTES, MAX_TEXT_LENGTH, cleanText, decodeImage,
};
