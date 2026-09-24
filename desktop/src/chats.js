'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { once } = require('node:events');
const { pipeline } = require('node:stream');

const MAX_MESSAGES = 500;
const MAX_TEXT_LENGTH = 2000;
const MAX_IMAGE_BYTES = 400 * 1024;
const MAX_IMAGE_SIDE = 8192;
const MAX_EMBED_URL = 2048;
const MAX_EMBED_TITLE = 200;
const MAX_EMBED_DESCRIPTION = 400;
const MAX_EMBED_SITE = 80;
const FILE_CIPHER = 'aes-256-ctr';
const FILE_READ_BYTES = 1024 * 1024;
const FILE_WRITE_BYTES = 1024 * 1024;
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

function embedText(value, max) {
  return typeof value === 'string' ? value.replace(/\s+/g, ' ').trim().slice(0, max) : '';
}

function decodeEmbed(embed) {
  if (!embed || typeof embed !== 'object' || typeof embed.url !== 'string' || embed.url.length > MAX_EMBED_URL) {
    return null;
  }
  let url;
  try {
    url = new URL(embed.url);
  } catch {
    return null;
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    return null;
  }
  const decoded = {
    url: url.href,
    title: embedText(embed.title, MAX_EMBED_TITLE),
    description: embedText(embed.description, MAX_EMBED_DESCRIPTION),
    siteName: embedText(embed.siteName, MAX_EMBED_SITE),
    image: embed.image ? decodeImage(embed.image) : null,
  };
  return decoded.title || decoded.description || decoded.image ? decoded : null;
}

function blobsOf(message) {
  return [message.image, message.file, message.embed && message.embed.image].filter((blob) => blob && blob.id);
}

// Arquivos grandes ficam cifrados em disco com uma chave so deles, guardada na conversa, que ja e
// protegida pelo sistema. Assim nada precisa caber inteiro na memoria.
class FileWriter {
  constructor(file) {
    this.file = file;
    this.temporary = `${file}.part`;
    this.key = crypto.randomBytes(32);
    this.iv = crypto.randomBytes(16);
    this.cipher = crypto.createCipheriv(FILE_CIPHER, this.key, this.iv);
    this.hash = crypto.createHash('sha256');
    this.size = 0;
    this.failure = null;
    this.draining = null;
    fs.mkdirSync(path.dirname(file), { recursive: true });
    this.stream = fs.createWriteStream(this.temporary, { highWaterMark: FILE_WRITE_BYTES });
    this.stream.on('error', (error) => {
      this.failure = error;
    });
  }

  write(chunk) {
    if (this.failure) {
      return Promise.reject(this.failure);
    }
    this.size += chunk.length;
    this.hash.update(chunk);
    if (this.stream.write(this.cipher.update(chunk))) {
      return Promise.resolve();
    }
    if (!this.draining) {
      this.draining = once(this.stream, 'drain').finally(() => {
        this.draining = null;
      });
    }
    return this.draining;
  }

  async finish(expected = null) {
    if (this.failure) {
      throw this.failure;
    }
    this.stream.end(this.cipher.final());
    await once(this.stream, 'close');
    if (this.failure) {
      throw this.failure;
    }
    const sha256 = this.hash.digest('hex');
    if (expected && (expected.size !== this.size || expected.sha256 !== sha256)) {
      await fs.promises.rm(this.temporary, { force: true });
      throw new Error('o arquivo chegou diferente do que foi enviado');
    }
    await fs.promises.rename(this.temporary, this.file);
    return {
      size: this.size,
      sha256,
      key: this.key.toString('base64'),
      iv: this.iv.toString('base64'),
    };
  }

  async abort() {
    if (!this.stream.closed) {
      const closed = once(this.stream, 'close').catch(() => {});
      this.stream.destroy();
      await closed;
    }
    await fs.promises.rm(this.temporary, { force: true }).catch(() => {});
  }
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

  fileDirectory() {
    return path.join(this.directory, 'files');
  }

  storedFile(conversationId, blobId) {
    return path.join(this.fileDirectory(), `${safeId(conversationId)}_${safeId(blobId)}.dat`);
  }

  createFileWriter(conversationId, blobId) {
    return new FileWriter(this.storedFile(conversationId, blobId));
  }

  hasFile(conversationId, blobId) {
    return fs.existsSync(this.storedFile(conversationId, blobId));
  }

  openFileReader(conversationId, blobId, secret) {
    const decipher = crypto.createDecipheriv(FILE_CIPHER, Buffer.from(secret.key, 'base64'),
      Buffer.from(secret.iv, 'base64'));
    const source = fs.createReadStream(this.storedFile(conversationId, blobId), { highWaterMark: FILE_READ_BYTES });
    return pipeline(source, decipher, () => {});
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
    const stored = this.storedFile(conversationId, blobId);
    fs.rmSync(stored, { force: true });
    fs.rmSync(`${stored}.part`, { force: true });
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
        for (const blob of blobsOf(old)) {
          this.removeBlob(conversationId, blob.id);
        }
      }
    }
    this.save(conversationId);
    return message;
  }

  find(conversationId, messageId) {
    return this.history(conversationId).find((message) => message.id === messageId) ?? null;
  }

  update(conversationId, messageId, change) {
    const message = this.find(conversationId, messageId);
    if (!message) {
      return null;
    }
    change(message);
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
    for (const directory of [this.blobDirectory(), this.fileDirectory()]) {
      try {
        for (const name of fs.readdirSync(directory)) {
          if (name.startsWith(prefix)) {
            fs.rmSync(path.join(directory, name), { force: true });
          }
        }
      } catch {
        // ainda nao existe pasta de anexos
      }
    }
  }
}

module.exports = {
  BLOB_ID_PATTERN, ChatStore, MAX_IMAGE_BYTES, MAX_TEXT_LENGTH, blobsOf, cleanText, decodeEmbed, decodeImage,
};
