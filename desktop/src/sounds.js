'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const MAX_SOUND_BYTES = 1024 * 1024;
const MAX_SOUNDS = 60;
const SOUND_ID_PATTERN = /^[a-f0-9]{16}$/;
const MIME_PATTERN = /^audio\/[\w.+-]+$/;

function cleanSoundName(name) {
  const clean = String(name ?? '').replace(/\s+/g, ' ').trim().slice(0, 32);
  return clean || 'Som';
}

class SoundStore {
  constructor(directory) {
    this.directory = directory;
    this.indexFile = path.join(directory, 'sounds.json');
    this.sounds = [];
  }

  load() {
    try {
      const data = JSON.parse(fs.readFileSync(this.indexFile, 'utf8'));
      this.sounds = Array.isArray(data.sounds)
        ? data.sounds
          .filter((sound) => sound && SOUND_ID_PATTERN.test(String(sound.id)) && MIME_PATTERN.test(String(sound.mime)))
          .map((sound) => ({
            id: sound.id,
            name: cleanSoundName(sound.name),
            mime: sound.mime,
            size: Number(sound.size) || 0,
            createdAt: Number(sound.createdAt) || 0,
          }))
        : [];
    } catch {
      this.sounds = [];
    }
    return this;
  }

  save() {
    fs.mkdirSync(this.directory, { recursive: true });
    const temporary = `${this.indexFile}.tmp`;
    fs.writeFileSync(temporary, JSON.stringify({ version: 1, sounds: this.sounds }, null, 2));
    fs.renameSync(temporary, this.indexFile);
  }

  fileFor(id) {
    return path.join(this.directory, `${id}.bin`);
  }

  list() {
    return this.sounds.map((sound) => ({ ...sound }));
  }

  add({ name, mime, data }) {
    if (this.sounds.length >= MAX_SOUNDS) {
      throw new Error(`o limite e de ${MAX_SOUNDS} sons`);
    }
    if (typeof mime !== 'string' || !MIME_PATTERN.test(mime)) {
      throw new Error('isso nao parece um arquivo de audio');
    }
    const buffer = Buffer.from(String(data ?? ''), 'base64');
    if (buffer.length === 0 || buffer.length > MAX_SOUND_BYTES) {
      throw new Error('o som precisa ter no maximo 1 MB');
    }
    const sound = {
      id: crypto.randomBytes(8).toString('hex'),
      name: cleanSoundName(name),
      mime: mime.slice(0, 60),
      size: buffer.length,
      createdAt: Date.now(),
    };
    fs.mkdirSync(this.directory, { recursive: true });
    fs.writeFileSync(this.fileFor(sound.id), buffer);
    this.sounds.push(sound);
    this.save();
    return { ...sound };
  }

  rename(id, name) {
    const sound = this.sounds.find((item) => item.id === id);
    if (!sound) {
      return false;
    }
    sound.name = cleanSoundName(name);
    this.save();
    return true;
  }

  remove(id) {
    const before = this.sounds.length;
    this.sounds = this.sounds.filter((item) => item.id !== id);
    if (this.sounds.length === before) {
      return false;
    }
    fs.rmSync(this.fileFor(id), { force: true });
    this.save();
    return true;
  }

  read(id) {
    if (!SOUND_ID_PATTERN.test(String(id)) || !this.sounds.some((item) => item.id === id)) {
      return null;
    }
    try {
      return fs.readFileSync(this.fileFor(id));
    } catch {
      return null;
    }
  }
}

module.exports = { MAX_SOUND_BYTES, SoundStore };
