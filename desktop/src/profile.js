'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { dataUrl, decodeAvatar } = require('./avatars');
const { cleanName } = require('./contacts');

const MEMBER_ID_PATTERN = /^[a-f0-9]{32}$/;
const MAX_RECENT_AVATARS = 6;

function pack(avatar) {
  return { mime: avatar.mime, data: avatar.data.toString('base64'), hash: avatar.hash };
}

function unpack(stored) {
  return stored ? decodeAvatar(stored.mime, stored.data) : null;
}

class ProfileStore {
  constructor(filePath, fallbackName) {
    this.filePath = filePath;
    this.fallbackName = cleanName(fallbackName, 'Telinha');
    this.profile = {
      memberId: '', name: this.fallbackName, avatar: null, recentAvatars: [],
    };
  }

  load() {
    let data = null;
    try {
      data = JSON.parse(fs.readFileSync(this.filePath, 'utf8'));
    } catch {
      data = null;
    }
    const valid = Boolean(data && MEMBER_ID_PATTERN.test(String(data.memberId)));
    const avatar = data ? unpack(data.avatar) : null;
    const recents = data && Array.isArray(data.recentAvatars)
      ? data.recentAvatars.map(unpack).filter(Boolean).slice(0, MAX_RECENT_AVATARS)
      : [];
    this.profile = {
      memberId: valid ? data.memberId : crypto.randomBytes(16).toString('hex'),
      name: cleanName(data && data.name, this.fallbackName),
      avatar: avatar ? pack(avatar) : null,
      recentAvatars: recents.map(pack),
    };
    if (!valid) {
      this.save();
    }
    return this;
  }

  save() {
    fs.mkdirSync(path.dirname(this.filePath), { recursive: true });
    const temporary = `${this.filePath}.tmp`;
    fs.writeFileSync(temporary, JSON.stringify({ version: 1, ...this.profile }, null, 2));
    fs.renameSync(temporary, this.filePath);
  }

  get memberId() {
    return this.profile.memberId;
  }

  get name() {
    return this.profile.name;
  }

  get avatar() {
    return this.profile.avatar;
  }

  get avatarHash() {
    return this.profile.avatar ? this.profile.avatar.hash : '';
  }

  avatarUrl() {
    const { avatar } = this.profile;
    return avatar ? dataUrl(avatar.mime, Buffer.from(avatar.data, 'base64')) : null;
  }

  recentAvatars() {
    return this.profile.recentAvatars.map((item) => ({
      hash: item.hash, url: dataUrl(item.mime, Buffer.from(item.data, 'base64')),
    }));
  }

  rename(name) {
    const clean = cleanName(name, this.profile.name);
    if (clean === this.profile.name) {
      return false;
    }
    this.profile.name = clean;
    this.save();
    return true;
  }

  setAvatar(avatar) {
    const packed = pack(avatar);
    this.profile.avatar = packed;
    this.profile.recentAvatars = [packed, ...this.profile.recentAvatars.filter((item) => item.hash !== packed.hash)]
      .slice(0, MAX_RECENT_AVATARS);
    this.save();
  }

  useRecent(hash) {
    const item = this.profile.recentAvatars.find((recent) => recent.hash === hash);
    if (!item) {
      return false;
    }
    this.profile.avatar = item;
    this.profile.recentAvatars = [item, ...this.profile.recentAvatars.filter((recent) => recent.hash !== hash)];
    this.save();
    return true;
  }

  removeAvatar() {
    if (!this.profile.avatar) {
      return false;
    }
    this.profile.avatar = null;
    this.save();
    return true;
  }
}

module.exports = { MEMBER_ID_PATTERN, ProfileStore };
