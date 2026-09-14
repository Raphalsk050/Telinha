'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {
  app, BrowserWindow, Notification, clipboard, dialog, ipcMain, safeStorage, session,
} = require('electron');
const {
  BLOB_ID_PATTERN, ChatStore, MAX_TEXT_LENGTH, decodeImage,
} = require('./src/chats');
const { ContactStore, cleanName } = require('./src/contacts');
const { AvatarStore, decodeAvatar } = require('./src/avatars');
const { MEMBER_ID_PATTERN, ProfileStore } = require('./src/profile');
const { ServerStore } = require('./src/servers');
const { PEER_ID_PATTERN, Signaling } = require('./src/signaling');
const { SoundStore } = require('./src/sounds');
const { StreamManager } = require('./src/streams');
const {
  MAX_FILE_BYTES, TransferInbox, cleanFileName, cleanMime, sha256, splitFile,
} = require('./src/transfers');
const {
  TelinhaSession,
  buildShareArgs,
  buildWatchArgs,
  isAudioDeviceId,
  listTargets,
  normalizeCode,
  resolveTelinhaExe,
} = require('./src/telinha-process');

const DM_ROOM = 'dm';
const AUDIO_SCOPES = new Set(['system', 'process', 'device', 'none']);
const ALLOWED_PERMISSIONS = new Set([
  'media', 'notifications', 'clipboard-sanitized-write', 'speaker-selection', 'fullscreen',
]);
const WEBRTC_PORT_MIN = 50020;
const WEBRTC_PORT_MAX = 50039;
const MAX_RTC_PAYLOAD = 60000;
const META_REPLY_INTERVAL_MS = 5000;
const CHANNEL_ID_PATTERN = /^[a-f0-9]{16}$/;

let mainWindow = null;
let profile = null;
let store = null;
let servers = null;
let chats = null;
let inbox = null;
let sounds = null;
let avatars = null;
let signaling = null;
let streams = null;
let manual = null;
let voice = null;
let quitting = false;
let lastLive = false;

const calls = new Map();
const metaReplies = new Map();
const avatarRequests = new Map();

const spaces = {
  list: () => [...store.list(), ...servers.list()].map(({ id }) => ({ id })),
  keysFor: (id) => store.keysFor(id) ?? servers.keysFor(id),
};

function telinhaExe() {
  const exe = resolveTelinhaExe({ resourcesPath: process.resourcesPath, isPackaged: app.isPackaged });
  if (!exe) {
    throw new Error('nao encontrei o telinha.exe, compile o projeto com cmake --build --preset msvc-release');
  }
  return exe;
}

function systemName() {
  try {
    return os.userInfo().username || 'Telinha';
  } catch {
    return 'Telinha';
  }
}

function protectedStorage() {
  const available = safeStorage.isEncryptionAvailable();
  return {
    encrypt: (text) => (available ? safeStorage.encryptString(text).toString('base64') : text),
    decrypt: (stored) => (available ? safeStorage.decryptString(Buffer.from(stored, 'base64')) : stored),
  };
}

function sendToWindow(channel, payload) {
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.webContents.send(channel, payload);
  }
}

function windowFocused() {
  return Boolean(mainWindow && !mainWindow.isDestroyed() && mainWindow.isFocused());
}

function notify(body) {
  if (!mainWindow || mainWindow.isDestroyed()) {
    return;
  }
  mainWindow.flashFrame(true);
  if (Notification.isSupported()) {
    const notification = new Notification({ title: 'Telinha', body });
    notification.on('click', () => {
      if (mainWindow.isMinimized()) {
        mainWindow.restore();
      }
      mainWindow.focus();
    });
    notification.show();
  }
}

function toLimit(value) {
  const number = Number(value);
  return Number.isFinite(number) && number > 0 ? Math.floor(number) : 0;
}

function conversationKey(spaceId, channelId) {
  return channelId ? `${spaceId}-${channelId}` : spaceId;
}

function profileView() {
  return {
    name: profile.name,
    memberId: profile.memberId,
    instanceId: signaling.instanceId,
    avatar: profile.avatarUrl(),
  };
}

function contactList() {
  return store.list()
    .map((contact) => ({ ...contact, online: signaling.isOnline(contact.id) }))
    .sort((a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name, 'pt-BR'));
}

function sendContacts() {
  sendToWindow('contacts:changed', contactList());
}

function sendServers() {
  sendToWindow('servers:changed', servers.list());
}

function sendVoice() {
  sendToWindow('voice:changed', voice ? { ...voice } : null);
}

/* presenca */

function presenceFor(spaceId) {
  const payload = { name: profile.name, memberId: profile.memberId, avatarHash: profile.avatarHash };
  if (voice && voice.spaceId === spaceId) {
    const { outgoing } = streams;
    const live = Boolean(outgoing && outgoing.state === 'live' && outgoing.spaceId === spaceId
      && outgoing.roomId === voice.roomId);
    payload.voice = {
      roomId: voice.roomId,
      mic: voice.mic,
      camera: voice.camera,
      deaf: voice.deaf,
      live,
      liveName: live ? outgoing.targetName : '',
    };
  }
  const meta = servers.meta(spaceId);
  if (meta) {
    payload.meta = meta;
  }
  return payload;
}

function replyMeta(spaceId) {
  const now = Date.now();
  if (now - (metaReplies.get(spaceId) ?? 0) < META_REPLY_INTERVAL_MS) {
    return;
  }
  metaReplies.set(spaceId, now);
  signaling.publishPresence(true, spaceId);
}

function handleAvatarPresence(spaceId, peerId, message) {
  const memberId = MEMBER_ID_PATTERN.test(String(message.memberId)) ? message.memberId : null;
  if (!memberId || memberId === profile.memberId) {
    return;
  }
  if (store.get(spaceId) && avatars.linkContact(spaceId, memberId)) {
    sendToWindow('avatars:changed', { memberId, contactId: spaceId, url: avatars.url(memberId) });
  }

  const hash = typeof message.avatarHash === 'string' ? message.avatarHash.slice(0, 64) : '';
  if (!hash) {
    if (avatars.remove(memberId)) {
      sendToWindow('avatars:changed', { memberId, url: null });
    }
    return;
  }
  if (avatars.hashFor(memberId) === hash) {
    return;
  }
  const key = `${memberId}:${hash}`;
  const now = Date.now();
  if (now - (avatarRequests.get(key) ?? 0) < 60000) {
    return;
  }
  avatarRequests.set(key, now);
  signaling.publish(spaceId, 'avatar-request', { to: peerId, hash });
}

function storeAvatar(spaceId, message) {
  const memberId = String(message.memberId ?? '');
  if (!MEMBER_ID_PATTERN.test(memberId) || memberId === profile.memberId) {
    return;
  }
  const peer = signaling.peers(spaceId).find((item) => item.id === message.from);
  if (peer && peer.memberId && peer.memberId !== memberId) {
    return;
  }
  const avatar = decodeAvatar(message.mime, message.data);
  if (!avatar || avatar.hash !== message.hash) {
    return;
  }
  if (avatars.put(memberId, avatar)) {
    sendToWindow('avatars:changed', { memberId, url: avatars.url(memberId) });
  }
}

function handlePeerPresence({ spaceId, peerId, message }) {
  handleAvatarPresence(spaceId, peerId, message);
  if (!servers.get(spaceId)) {
    return;
  }
  let changed = false;
  if (message.meta) {
    const result = servers.applyMeta(spaceId, message.meta);
    changed = result.changed;
    if (result.mineNewer) {
      replyMeta(spaceId);
    }
    if (result.changed) {
      leaveRemovedRoom(spaceId);
    }
  }
  changed = servers.touchMember(spaceId, message.memberId, message.name) || changed;
  if (changed) {
    sendServers();
  }
}

function handlePeers({ spaceId, peers }) {
  sendToWindow('presence:changed', { spaceId, peers });
  if (voice && voice.spaceId === spaceId) {
    streams.reconcile(spaceId, voice.roomId, peers);
  }

  const contact = store.get(spaceId);
  if (!contact) {
    return;
  }
  const inCall = peers.filter((peer) => peer.voice && peer.voice.roomId === DM_ROOM);
  let next = 'none';
  if (inCall.length > 0) {
    next = inCall.some((peer) => peer.voice.live) ? 'live' : 'call';
  }
  const previous = calls.get(spaceId) ?? 'none';
  calls.set(spaceId, next);

  const joined = Boolean(voice && voice.spaceId === spaceId);
  if (joined || next === 'none' || next === previous || (previous === 'live' && next === 'call')) {
    return;
  }
  sendToWindow('call:incoming', { contactId: spaceId, name: contact.name, live: next === 'live' });
  notify(next === 'live'
    ? `${contact.name} está transmitindo a tela para você`
    : `${contact.name} está chamando você`);
}

/* mensagens */

function chatTarget(spaceId, message) {
  const contact = store.get(spaceId);
  const server = contact ? null : servers.get(spaceId);
  if ((!contact && !server) || typeof message.messageId !== 'string' || message.messageId.length > 64) {
    return null;
  }
  const channelId = server ? String(message.channelId ?? '') : null;
  if (server && !CHANNEL_ID_PATTERN.test(channelId)) {
    return null;
  }
  return { contact, server, channelId, key: conversationKey(spaceId, channelId) };
}

function storedConversation(spaceId, channelId) {
  if (store.get(spaceId)) {
    return spaceId;
  }
  const server = servers.get(spaceId);
  return server && server.channels.some((channel) => channel.id === channelId)
    ? conversationKey(spaceId, channelId)
    : null;
}

function storeIncomingChat(spaceId, message, fileData) {
  const target = chatTarget(spaceId, message);
  if (!target) {
    return;
  }
  const {
    contact, server, channelId, key,
  } = target;
  const text = typeof message.text === 'string' ? message.text.slice(0, MAX_TEXT_LENGTH) : '';
  const image = message.image ? decodeImage(message.image) : null;
  if (!text && !image && !fileData) {
    return;
  }

  const now = Date.now();
  const author = contact ? contact.name : cleanName(message.author, 'Alguém');
  const imageInfo = image
    ? { id: crypto.randomUUID(), mime: image.mime, width: image.width, height: image.height }
    : undefined;
  const fileInfo = fileData
    ? {
      id: crypto.randomUUID(),
      name: cleanFileName(message.file.name),
      mime: cleanMime(message.file.mime),
      size: fileData.length,
    }
    : undefined;
  if (imageInfo) {
    chats.saveBlob(key, imageInfo.id, image.data);
  }
  if (fileInfo) {
    chats.saveBlob(key, fileInfo.id, fileData);
  }
  const stored = chats.append(key, {
    id: message.messageId,
    author,
    memberId: typeof message.memberId === 'string' ? message.memberId.slice(0, 32) : null,
    mine: false,
    text,
    sentAt: Number.isFinite(message.sentAt) ? Math.min(message.sentAt, now + 60000) : now,
    image: imageInfo,
    file: fileInfo,
  });
  if (!stored) {
    for (const blob of [imageInfo, fileInfo]) {
      if (blob) {
        chats.removeBlob(key, blob.id);
      }
    }
    return;
  }
  sendToWindow('chat:message', { spaceId, channelId, message: stored });

  if (!windowFocused()) {
    const channel = server ? server.channels.find((item) => item.id === channelId) : null;
    const where = server ? ` em ${server.name}${channel ? ` #${channel.name}` : ''}` : '';
    let summary = 'mandou uma imagem';
    if (stored.text) {
      summary = stored.text.slice(0, 120);
    } else if (fileInfo) {
      summary = `mandou ${fileInfo.name}`;
    }
    notify(`${author}${where}: ${summary}`);
  }
}

function handleChat(spaceId, message) {
  if (!chatTarget(spaceId, message)) {
    return;
  }
  if (message.file) {
    inbox.addMessage(spaceId, message);
    return;
  }
  storeIncomingChat(spaceId, message, null);
}

function publishFile(spaceId, payload, fileInfo, data) {
  const transferId = crypto.randomUUID();
  const chunks = splitFile(data);
  for (const [index, chunk] of chunks.entries()) {
    const sent = signaling.publish(spaceId, 'file-chunk', {
      transferId, index, total: chunks.length, data: chunk.toString('base64'),
    });
    if (!sent) {
      return false;
    }
  }
  return signaling.publish(spaceId, 'chat', {
    ...payload,
    file: {
      transferId, name: fileInfo.name, mime: fileInfo.mime, size: data.length, sha256: sha256(data),
    },
  });
}

function handleSignalingMessage({ spaceId, message }) {
  if (message.type === 'avatar-request') {
    if (message.to === signaling.instanceId && profile.avatar && message.hash === profile.avatarHash) {
      signaling.publish(spaceId, 'avatar', {
        to: message.from,
        memberId: profile.memberId,
        hash: profile.avatarHash,
        mime: profile.avatar.mime,
        data: profile.avatar.data,
      });
    }
    return;
  }
  if (message.type === 'avatar') {
    if (message.to === signaling.instanceId) {
      storeAvatar(spaceId, message);
    }
    return;
  }
  if (message.type === 'file-chunk') {
    inbox.addChunk(spaceId, message);
    return;
  }
  if (message.type === 'chat') {
    handleChat(spaceId, message);
    return;
  }
  if (message.type === 'rtc') {
    if (message.to === signaling.instanceId && voice && voice.spaceId === spaceId
      && message.roomId === voice.roomId) {
      sendToWindow('rtc:message', { spaceId, from: message.from, payload: message.payload });
    }
    return;
  }
  if (message.type.startsWith('stream-')) {
    streams.handleMessage(spaceId, message);
  }
}

/* chamada */

function roomExists(spaceId, roomId) {
  if (store.get(spaceId)) {
    return roomId === DM_ROOM;
  }
  const server = servers.get(spaceId);
  return Boolean(server && server.channels.some((channel) => channel.id === roomId && channel.kind === 'voice'));
}

function setVoice(next) {
  const previous = voice;
  voice = next;
  if (previous && (!next || previous.spaceId !== next.spaceId)) {
    signaling.publishPresence(true, previous.spaceId);
  }
  if (next) {
    signaling.publishPresence(true, next.spaceId);
  }
  sendVoice();
}

function leaveVoice() {
  if (!voice) {
    return;
  }
  streams.leaveRoom();
  setVoice(null);
}

function leaveRemovedRoom(spaceId) {
  if (voice && voice.spaceId === spaceId && !roomExists(spaceId, voice.roomId)) {
    leaveVoice();
  }
}

/* conexao com codigo */

function startManual(request) {
  if (manual && !manual.session.exited) {
    throw new Error('ja existe uma conexao com codigo em andamento');
  }
  const role = request.role === 'share' ? 'share' : 'watch';
  const args = role === 'share'
    ? buildShareArgs({ ...request, excludePid: process.pid })
    : buildWatchArgs({});
  const telinha = new TelinhaSession(telinhaExe(), args);
  const current = { session: telinha, role, codes: { invite: null, answer: null }, connected: false };
  manual = current;

  telinha.on('event', (event) => {
    handleManualEvent(current, event);
    sendToWindow('telinha:event', event);
  });
  telinha.on('log', (line) => sendToWindow('telinha:log', line));
  telinha.on('exit', (info) => {
    if (manual === current) {
      manual = null;
    }
    sendToWindow('telinha:exit', info);
  });
  telinha.start();
  sendToWindow('session:started', { role, mode: 'manual', contactId: null, contactName: null });
  return { role };
}

function handleManualEvent(current, event) {
  if (event.event === 'code') {
    current.codes[event.kind === 'invite' ? 'invite' : 'answer'] = normalizeCode(event.code);
    return;
  }
  if (event.event !== 'state' || event.state !== 'Connected' || current.connected) {
    return;
  }
  current.connected = true;
  const { invite, answer } = current.codes;
  if (!invite || !answer) {
    return;
  }
  const name = `Contato de ${new Date().toLocaleDateString('pt-BR')}`;
  const { contact, created } = store.saveFromCodes(invite, answer, name);
  if (created) {
    signaling.addContact(contact.id);
  }
  sendContacts();
  sendToWindow('contact:saved', { contact, created });
}

function manualCommand(role, command) {
  return Boolean(manual && manual.role === role && manual.connected && manual.session.sendCommand(command));
}

function targetCommand(target) {
  return {
    command: 'switch_target',
    kind: target && ['window', 'device'].includes(target.kind) ? target.kind : 'monitor',
    handle: toLimit(target && target.handle),
  };
}

function qualityCommand(quality) {
  const value = quality ?? {};
  return {
    command: 'set_quality',
    max_fps: toLimit(value.maxFps),
    max_width: toLimit(value.maxWidth),
    max_height: toLimit(value.maxHeight),
    max_bitrate_kbps: toLimit(value.maxBitrateKbps),
  };
}

function audioCommand(audio) {
  const scope = audio && AUDIO_SCOPES.has(audio.scope) ? audio.scope : null;
  if (scope === 'device') {
    return isAudioDeviceId(audio.device) ? { command: 'set_audio', scope, pid: 0, device: audio.device } : null;
  }
  return scope ? { command: 'set_audio', scope, pid: toLimit(audio.pid) } : null;
}

/* ipc */

function registerIpc() {
  ipcMain.handle('app:profile', () => profileView());
  ipcMain.handle('profile:set-avatar', (_event, request) => {
    const avatar = decodeAvatar(request && request.mime, request && request.data);
    if (!avatar) {
      throw new Error('essa imagem nao pode ser usada como avatar');
    }
    profile.setAvatar(avatar);
    signaling.publishPresence(true);
    sendToWindow('profile:changed', profileView());
    return profileView();
  });
  ipcMain.handle('profile:remove-avatar', () => {
    if (profile.removeAvatar()) {
      signaling.publishPresence(true);
      sendToWindow('profile:changed', profileView());
    }
    return profileView();
  });
  ipcMain.handle('profile:recent-avatars', () => profile.recentAvatars());
  ipcMain.handle('profile:use-recent-avatar', (_event, hash) => {
    if (profile.useRecent(String(hash))) {
      signaling.publishPresence(true);
      sendToWindow('profile:changed', profileView());
    }
    return profileView();
  });
  ipcMain.handle('avatars:snapshot', () => avatars.snapshot());
  ipcMain.handle('app:set-name', (_event, name) => {
    if (profile.rename(name)) {
      signaling.profileName = profile.name;
      for (const server of servers.list()) {
        servers.touchMember(server.id, profile.memberId, profile.name);
      }
      signaling.publishPresence(true);
      sendServers();
      sendToWindow('profile:changed', profileView());
    }
    return profileView();
  });

  ipcMain.handle('telinha:list', () => listTargets(telinhaExe()));
  ipcMain.handle('telinha:start', (_event, request) => startManual(request ?? {}));
  ipcMain.handle('telinha:submit-code', (_event, code) => {
    if (!manual) {
      return false;
    }
    const accepted = manual.session.submitCode(code);
    if (accepted) {
      manual.codes[manual.role === 'share' ? 'answer' : 'invite'] = normalizeCode(code);
    }
    return accepted;
  });
  ipcMain.handle('telinha:stop', () => {
    if (manual) {
      manual.session.stop();
    }
  });
  ipcMain.handle('telinha:switch-target', (_event, target) => manualCommand('share', targetCommand(target)));
  ipcMain.handle('telinha:set-quality', (_event, quality) => manualCommand('share', qualityCommand(quality)));
  ipcMain.handle('telinha:set-audio', (_event, audio) => {
    const command = audioCommand(audio);
    return Boolean(command) && manualCommand('share', command);
  });
  ipcMain.handle('telinha:set-fullscreen', (_event, enabled) => manualCommand('watch', {
    command: 'set_fullscreen', enabled: Boolean(enabled),
  }));

  ipcMain.handle('contacts:list', () => contactList());
  ipcMain.handle('contacts:rename', (_event, { id, name }) => {
    const renamed = store.rename(id, name);
    sendContacts();
    return renamed;
  });
  ipcMain.handle('contacts:remove', (_event, id) => {
    if (voice && voice.spaceId === id) {
      leaveVoice();
    }
    signaling.removeSpace(id);
    const removed = store.remove(id);
    chats.forget(id);
    calls.delete(id);
    sendContacts();
    return removed;
  });

  ipcMain.handle('servers:list', () => servers.list());
  ipcMain.handle('servers:create', (_event, name) => {
    const server = servers.create(name, { memberId: profile.memberId, name: profile.name });
    signaling.addSpace(server.id);
    sendServers();
    return server;
  });
  ipcMain.handle('servers:join', (_event, code) => {
    const result = servers.join(code, { memberId: profile.memberId, name: profile.name });
    if (!result) {
      throw new Error('esse convite nao e valido, copie o texto inteiro que comeca com TELINHA-GRUPO.');
    }
    if (result.created) {
      signaling.addSpace(result.server.id);
    }
    sendServers();
    return result;
  });
  ipcMain.handle('servers:invite', (_event, id) => servers.inviteCode(id));
  ipcMain.handle('servers:rename', (_event, { id, name }) => {
    const renamed = servers.rename(id, name);
    if (renamed) {
      sendServers();
      signaling.publishPresence(true, id);
    }
    return renamed;
  });
  ipcMain.handle('servers:leave', (_event, id) => {
    const server = servers.get(id);
    if (!server) {
      return false;
    }
    if (voice && voice.spaceId === id) {
      leaveVoice();
    }
    signaling.removeSpace(id);
    for (const channel of server.channels) {
      chats.forget(conversationKey(id, channel.id));
    }
    servers.leave(id);
    sendServers();
    return true;
  });
  ipcMain.handle('servers:add-channel', (_event, { serverId, kind, name }) => {
    const channel = servers.addChannel(serverId, kind, name);
    if (channel) {
      sendServers();
      signaling.publishPresence(true, serverId);
    }
    return channel;
  });
  ipcMain.handle('servers:rename-channel', (_event, { serverId, channelId, name }) => {
    const renamed = servers.renameChannel(serverId, channelId, name);
    if (renamed) {
      sendServers();
      signaling.publishPresence(true, serverId);
    }
    return renamed;
  });
  ipcMain.handle('servers:remove-channel', (_event, { serverId, channelId }) => {
    const removed = servers.removeChannel(serverId, channelId);
    if (removed) {
      chats.forget(conversationKey(serverId, channelId));
      leaveRemovedRoom(serverId);
      sendServers();
      signaling.publishPresence(true, serverId);
    }
    return removed;
  });

  ipcMain.handle('presence:snapshot', () => signaling.snapshot());

  ipcMain.handle('chat:history', (_event, { spaceId, channelId = null }) => {
    if (store.get(spaceId)) {
      return chats.history(spaceId);
    }
    const server = servers.get(spaceId);
    if (server && server.channels.some((channel) => channel.id === channelId)) {
      return chats.history(conversationKey(spaceId, channelId));
    }
    return [];
  });
  ipcMain.handle('chat:send', (_event, {
    spaceId, channelId = null, text, image = null, file = null,
  }) => {
    const contact = store.get(spaceId);
    const server = contact ? null : servers.get(spaceId);
    if (!contact && !server) {
      throw new Error('conversa nao encontrada');
    }
    if (server && !server.channels.some((channel) => channel.id === channelId)) {
      throw new Error('esse canal nao existe mais');
    }
    const decoded = image ? decodeImage(image) : null;
    if (image && !decoded) {
      throw new Error('essa imagem nao pode ser enviada, tente outra');
    }
    const fileData = file ? Buffer.from(String(file.data ?? ''), 'base64') : null;
    if (fileData && (fileData.length === 0 || fileData.length > MAX_FILE_BYTES)) {
      throw new Error('o arquivo precisa ter no maximo 8 MB');
    }
    const message = chats.createOutgoing(profile.name, text, { allowEmpty: Boolean(decoded || fileData) });
    if (!message) {
      return null;
    }

    const key = conversationKey(spaceId, server ? channelId : null);
    const imageInfo = decoded
      ? { id: crypto.randomUUID(), mime: decoded.mime, width: decoded.width, height: decoded.height }
      : undefined;
    const fileInfo = fileData
      ? {
        id: crypto.randomUUID(), name: cleanFileName(file.name), mime: cleanMime(file.mime), size: fileData.length,
      }
      : undefined;
    if (imageInfo) {
      chats.saveBlob(key, imageInfo.id, decoded.data);
    }
    if (fileInfo) {
      chats.saveBlob(key, fileInfo.id, fileData);
    }

    const payload = {
      messageId: message.id,
      text: message.text,
      sentAt: message.sentAt,
      channelId: server ? channelId : undefined,
      author: profile.name,
      memberId: profile.memberId,
      image: decoded ? {
        mime: decoded.mime,
        width: decoded.width,
        height: decoded.height,
        data: decoded.data.toString('base64'),
      } : undefined,
    };
    const delivered = fileInfo
      ? publishFile(spaceId, payload, fileInfo, fileData)
      : signaling.publish(spaceId, 'chat', payload);
    return chats.append(key, {
      ...message, memberId: profile.memberId, delivered, image: imageInfo, file: fileInfo,
    });
  });

  ipcMain.handle('chat:image', (_event, { spaceId, channelId = null, imageId }) => {
    const key = storedConversation(spaceId, channelId);
    if (!key || !BLOB_ID_PATTERN.test(String(imageId))) {
      return null;
    }
    const message = chats.history(key).find((item) => item.image && item.image.id === imageId);
    const data = message ? chats.loadBlob(key, imageId) : null;
    return data ? `data:${message.image.mime};base64,${data.toString('base64')}` : null;
  });

  ipcMain.handle('chat:save-file', async (_event, { spaceId, channelId = null, fileId }) => {
    const key = storedConversation(spaceId, channelId);
    if (!key || !BLOB_ID_PATTERN.test(String(fileId))) {
      return false;
    }
    const message = chats.history(key).find((item) => item.file && item.file.id === fileId);
    const data = message ? chats.loadBlob(key, fileId) : null;
    if (!data) {
      throw new Error('esse arquivo nao esta mais guardado neste computador');
    }
    const result = await dialog.showSaveDialog(mainWindow, {
      defaultPath: path.join(app.getPath('downloads'), cleanFileName(message.file.name)),
    });
    if (result.canceled || !result.filePath) {
      return false;
    }
    await fs.promises.writeFile(result.filePath, data);
    return true;
  });

  ipcMain.handle('voice:join', (_event, request) => {
    const { spaceId, roomId } = request ?? {};
    if (!roomExists(spaceId, roomId)) {
      throw new Error('essa sala nao existe mais');
    }
    if (voice && (voice.spaceId !== spaceId || voice.roomId !== roomId)) {
      streams.leaveRoom();
    }
    setVoice({
      spaceId,
      roomId,
      mic: request.mic !== false,
      camera: request.camera === true,
      deaf: request.deaf === true,
    });
    return { ...voice };
  });
  ipcMain.handle('voice:update', (_event, changes) => {
    if (!voice) {
      return null;
    }
    const value = changes ?? {};
    setVoice({
      ...voice,
      mic: typeof value.mic === 'boolean' ? value.mic : voice.mic,
      camera: typeof value.camera === 'boolean' ? value.camera : voice.camera,
      deaf: typeof value.deaf === 'boolean' ? value.deaf : voice.deaf,
    });
    return { ...voice };
  });
  ipcMain.handle('voice:leave', () => leaveVoice());

  ipcMain.handle('rtc:send', (_event, { spaceId, to, payload }) => {
    if (!voice || voice.spaceId !== spaceId || !PEER_ID_PATTERN.test(String(to))) {
      return false;
    }
    if (JSON.stringify(payload ?? null).length > MAX_RTC_PAYLOAD) {
      return false;
    }
    return signaling.publish(spaceId, 'rtc', { to, roomId: voice.roomId, payload });
  });

  ipcMain.handle('stream:go-live', (_event, { share, targetName }) => {
    if (!voice) {
      throw new Error('entre numa chamada antes de transmitir');
    }
    streams.goLive({
      spaceId: voice.spaceId,
      roomId: voice.roomId,
      share: share ?? {},
      targetName: cleanName(targetName, 'Tela'),
    });
    return streams.snapshot();
  });
  ipcMain.handle('stream:stop', () => streams.stopLive());
  ipcMain.handle('stream:switch-target', (_event, target) => streams.command(targetCommand(target)));
  ipcMain.handle('stream:set-quality', (_event, quality) => streams.command(qualityCommand(quality)));
  ipcMain.handle('stream:set-audio', (_event, audio) => {
    const command = audioCommand(audio);
    return Boolean(command) && streams.command(command);
  });
  ipcMain.handle('stream:watch', (_event, sharerId) => {
    if (!voice) {
      throw new Error('entre na chamada para assistir');
    }
    const sharer = signaling.peers(voice.spaceId).find((peer) => peer.id === sharerId && peer.voice
      && peer.voice.roomId === voice.roomId && peer.voice.live);
    if (!sharer) {
      throw new Error('essa transmissao ja terminou');
    }
    const contact = store.get(voice.spaceId);
    streams.watch({
      spaceId: voice.spaceId,
      roomId: voice.roomId,
      sharerId,
      name: contact ? contact.name : (sharer.name || 'Alguém'),
    });
    return streams.snapshot();
  });
  ipcMain.handle('stream:unwatch', (_event, sharerId) => streams.unwatch(sharerId));
  ipcMain.handle('stream:fullscreen', (_event, { sharerId, enabled }) => streams.setFullscreen(sharerId, enabled));
  ipcMain.handle('stream:volume', (_event, { sharerId, volume }) => streams.setVolume(sharerId, volume));
  ipcMain.handle('stream:answer', (_event, { sharerId, sdp }) => streams.answerEmbedded(sharerId, sdp));
  ipcMain.handle('stream:embedded-state', (_event, { sharerId, state }) => streams.embeddedState(sharerId, state));
  ipcMain.handle('stream:popout', (_event, sharerId) => {
    const entry = streams.incoming.get(sharerId);
    if (entry) {
      streams.switchMode(entry, 'native');
    }
    return Boolean(entry);
  });
  ipcMain.handle('stream:popin', (_event, sharerId) => {
    const entry = streams.incoming.get(sharerId);
    if (entry) {
      streams.switchMode(entry, 'embedded');
    }
    return Boolean(entry);
  });
  ipcMain.handle('streams:state', () => streams.snapshot());

  ipcMain.handle('sounds:list', () => sounds.list());
  ipcMain.handle('sounds:add', (_event, sound) => sounds.add(sound ?? {}));
  ipcMain.handle('sounds:rename', (_event, { id, name }) => sounds.rename(id, name));
  ipcMain.handle('sounds:remove', (_event, id) => sounds.remove(id));
  ipcMain.handle('sounds:data', (_event, id) => {
    const data = sounds.read(id);
    return data ? data.toString('base64') : null;
  });

  ipcMain.handle('signaling:status', () => signaling.status());
  ipcMain.handle('clipboard:write', (_event, text) => {
    clipboard.writeText(String(text));
  });
  ipcMain.handle('clipboard:read', () => clipboard.readText());
}

function configurePermissions() {
  const current = session.defaultSession;
  current.setPermissionRequestHandler((_webContents, permission, callback) => {
    callback(ALLOWED_PERMISSIONS.has(permission));
  });
  current.setPermissionCheckHandler((_webContents, permission) => ALLOWED_PERMISSIONS.has(permission));
}

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1360,
    height: 860,
    minWidth: 1040,
    minHeight: 640,
    title: 'Telinha',
    backgroundColor: '#1e1f22',
    autoHideMenuBar: true,
    show: false,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      backgroundThrottling: false,
    },
  });
  const contents = mainWindow.webContents;
  if (typeof contents.setWebRTCUDPPortRange === 'function') {
    contents.setWebRTCUDPPortRange({ min: WEBRTC_PORT_MIN, max: WEBRTC_PORT_MAX });
  }
  contents.setWindowOpenHandler(() => ({ action: 'deny' }));
  contents.on('will-navigate', (event) => event.preventDefault());
  mainWindow.once('ready-to-show', () => mainWindow.show());
  mainWindow.loadFile(path.join(__dirname, 'renderer', 'index.html'));
}

app.setAppUserModelId('dev.telinha.desktop');

app.whenReady().then(() => {
  const storage = protectedStorage();
  const userData = app.getPath('userData');
  profile = new ProfileStore(path.join(userData, 'profile.json'), systemName()).load();
  store = new ContactStore(path.join(userData, 'contacts.json'), storage).load();
  servers = new ServerStore(path.join(userData, 'servers.json'), storage).load();
  chats = new ChatStore(path.join(userData, 'chats'), storage);
  inbox = new TransferInbox({ onComplete: storeIncomingChat });
  sounds = new SoundStore(path.join(userData, 'sounds')).load();
  avatars = new AvatarStore(path.join(userData, 'avatars')).load();

  signaling = new Signaling({ spaces, profileName: profile.name, presenceFor });
  streams = new StreamManager({
    exe: telinhaExe,
    publish: (spaceId, type, body) => signaling.publish(spaceId, type, body),
    selfId: signaling.instanceId,
    ownerPid: process.pid,
  });

  signaling.on('message', handleSignalingMessage);
  signaling.on('presence', sendContacts);
  signaling.on('peers', handlePeers);
  signaling.on('peer-presence', handlePeerPresence);
  signaling.on('peer-name', ({ spaceId, name }) => {
    if (store.adoptRemoteName(spaceId, name)) {
      sendContacts();
    }
  });
  signaling.on('status', (status) => sendToWindow('signaling:status', status));

  streams.on('changed', (snapshot) => {
    sendToWindow('streams:changed', snapshot);
    const live = Boolean(snapshot.outgoing && snapshot.outgoing.state === 'live');
    if (live !== lastLive) {
      lastLive = live;
      if (voice) {
        signaling.publishPresence(true, voice.spaceId);
      }
    }
  });
  streams.on('event', (payload) => sendToWindow('stream:event', payload));
  streams.on('log', (payload) => sendToWindow('stream:log', payload));
  streams.on('outgoing-ended', (info) => sendToWindow('stream:outgoing-ended', info));
  streams.on('incoming-ended', (info) => sendToWindow('stream:incoming-ended', info));
  streams.on('embedded-offer', (payload) => sendToWindow('stream:offer', payload));

  configurePermissions();
  registerIpc();
  createWindow();
  signaling.start();
});

app.on('before-quit', (event) => {
  if (quitting || !signaling) {
    return;
  }
  quitting = true;
  event.preventDefault();

  if (manual) {
    manual.session.stop();
  }
  streams.stopAll();
  Promise.race([signaling.stop(), new Promise((resolve) => setTimeout(resolve, 1500))])
    .finally(() => app.quit());
});

app.on('window-all-closed', () => app.quit());
