'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { dataUrl, decodeAvatar } = require('./avatars');
const { cleanName } = require('./contacts');

const MEMBER_ID_PATTERN = /^[a-f0-9]{32}$/;
const MAX_RECENT_AVATARS = 6;
const HEX_PATTERN = /^#[0-9a-f]{6}$/i;
const MAX_BIO_LENGTH = 190;
const MAX_BIO_LINES = 6;
// Quantas cores cada efeito de nome leva, no minimo e no maximo.
const NAME_EFFECTS = {
  solid: [1, 1], neon: [1, 1], gradient: [2, 2], prism: [2, 5],
};

function hexColors(list, max) {
  return (Array.isArray(list) ? list : [])
    .filter((color) => typeof color === 'string' && HEX_PATTERN.test(color))
    .slice(0, max)
    .map((color) => color.toLowerCase());
}

function cleanBio(value) {
  if (typeof value !== 'string') {
    return '';
  }
  return value.replace(/\r/g, '').split('\n').slice(0, MAX_BIO_LINES).join('\n')
    .slice(0, MAX_BIO_LENGTH)
    .trim();
}

// Estilo do perfil: o efeito do nome, as duas cores do cartao e o "sobre mim". O dos outros chega
// pela presenca, entao o que nao couber no formato cai fora.
function cleanStyle(candidate) {
  const source = candidate && typeof candidate === 'object' ? candidate : {};
  const wanted = source.name && typeof source.name === 'object' ? source.name : {};
  const limits = Object.hasOwn(NAME_EFFECTS, wanted.effect) ? NAME_EFFECTS[wanted.effect] : null;
  const colors = limits ? hexColors(wanted.colors, limits[1]) : [];
  const theme = hexColors(source.theme, 2);
  return {
    name: limits && colors.length >= limits[0] ? { effect: wanted.effect, colors } : null,
    theme: theme.length === 2 ? theme : null,
    bio: cleanBio(source.bio),
  };
}

function hasStyle(style) {
  return Boolean(style && (style.name || style.theme || style.bio));
}

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
      memberId: '', name: this.fallbackName, avatar: null, banner: null, recentAvatars: [], style: cleanStyle(null),
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
    const banner = data ? unpack(data.banner) : null;
    const recents = data && Array.isArray(data.recentAvatars)
      ? data.recentAvatars.map(unpack).filter(Boolean).slice(0, MAX_RECENT_AVATARS)
      : [];
    this.profile = {
      memberId: valid ? data.memberId : crypto.randomBytes(16).toString('hex'),
      name: cleanName(data && data.name, this.fallbackName),
      avatar: avatar ? pack(avatar) : null,
      banner: banner ? pack(banner) : null,
      recentAvatars: recents.map(pack),
      style: cleanStyle(data && data.style),
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

  get style() {
    return this.profile.style;
  }

  setStyle(style) {
    const clean = cleanStyle(style);
    if (JSON.stringify(clean) === JSON.stringify(this.profile.style)) {
      return false;
    }
    this.profile.style = clean;
    this.save();
    return true;
  }

  get avatarHash() {
    return this.profile.avatar ? this.profile.avatar.hash : '';
  }

  avatarUrl() {
    const { avatar } = this.profile;
    return avatar ? dataUrl(avatar.mime, Buffer.from(avatar.data, 'base64')) : null;
  }

  get banner() {
    return this.profile.banner;
  }

  get bannerHash() {
    return this.profile.banner ? this.profile.banner.hash : '';
  }

  bannerUrl() {
    const { banner } = this.profile;
    return banner ? dataUrl(banner.mime, Buffer.from(banner.data, 'base64')) : null;
  }

  // A imagem do banner passa pela mesma conferencia do avatar, com o mesmo limite de tamanho.
  setBanner(banner) {
    this.profile.banner = pack(banner);
    this.save();
  }

  removeBanner() {
    if (!this.profile.banner) {
      return false;
    }
    this.profile.banner = null;
    this.save();
    return true;
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

module.exports = {
  MEMBER_ID_PATTERN, ProfileStore, cleanStyle, hasStyle,
};
