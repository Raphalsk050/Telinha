'use strict';

const { contextBridge, ipcRenderer, webUtils } = require('electron');

const MAX_LOOSE_FILE_BYTES = 64 * 1024 * 1024;

function subscribe(channel, listener) {
  const handler = (_event, payload) => listener(payload);
  ipcRenderer.on(channel, handler);
  return () => ipcRenderer.removeListener(channel, handler);
}

const invoke = (channel, payload) => ipcRenderer.invoke(channel, payload);

// O arquivo chega aqui como File. O caminho dele no disco e resolvido neste ponto e vai direto
// para o processo principal, sem nunca passar pela pagina. Sem caminho (arquivo que so existe na
// memoria), os bytes vao juntos, com um limite.
async function outgoingFile(file) {
  if (!file || !file.source) {
    return null;
  }
  const { source } = file;
  let filePath = '';
  try {
    filePath = webUtils.getPathForFile(source);
  } catch {
    filePath = '';
  }
  const described = { name: String(file.name ?? ''), mime: String(file.mime ?? '') };
  if (filePath) {
    return { ...described, path: filePath };
  }
  if (typeof source.arrayBuffer !== 'function' || source.size > MAX_LOOSE_FILE_BYTES) {
    throw new Error('esse arquivo não está numa pasta do computador. Salve ele numa pasta e anexe de lá.');
  }
  return { ...described, bytes: new Uint8Array(await source.arrayBuffer()) };
}

contextBridge.exposeInMainWorld('telinha', {
  profile: () => invoke('app:profile'),
  setName: (name) => invoke('app:set-name', name),
  setAvatar: (avatar) => invoke('profile:set-avatar', avatar),
  removeAvatar: () => invoke('profile:remove-avatar'),
  recentAvatars: () => invoke('profile:recent-avatars'),
  useRecentAvatar: (hash) => invoke('profile:use-recent-avatar', hash),
  avatarsSnapshot: () => invoke('avatars:snapshot'),
  onAvatars: (listener) => subscribe('avatars:changed', listener),

  listTargets: () => invoke('telinha:list'),
  startShare: (options) => invoke('telinha:start', { ...options, role: 'share' }),
  startWatch: (options) => invoke('telinha:start', { ...options, role: 'watch' }),
  submitCode: (code) => invoke('telinha:submit-code', code),
  stop: () => invoke('telinha:stop'),
  switchTarget: (target) => invoke('telinha:switch-target', target),
  setQuality: (quality) => invoke('telinha:set-quality', quality),
  setAudio: (audio) => invoke('telinha:set-audio', audio),
  setFullscreen: (enabled) => invoke('telinha:set-fullscreen', enabled),

  listContacts: () => invoke('contacts:list'),
  renameContact: (id, name) => invoke('contacts:rename', { id, name }),
  removeContact: (id) => invoke('contacts:remove', id),

  listServers: () => invoke('servers:list'),
  createServer: (name) => invoke('servers:create', name),
  joinServer: (code) => invoke('servers:join', code),
  inviteCode: (id) => invoke('servers:invite', id),
  renameServer: (id, name) => invoke('servers:rename', { id, name }),
  leaveServer: (id) => invoke('servers:leave', id),
  addChannel: (serverId, kind, name) => invoke('servers:add-channel', { serverId, kind, name }),
  renameChannel: (serverId, channelId, name) => invoke('servers:rename-channel', { serverId, channelId, name }),
  removeChannel: (serverId, channelId) => invoke('servers:remove-channel', { serverId, channelId }),

  presenceSnapshot: () => invoke('presence:snapshot'),
  chatHistory: (spaceId, channelId) => invoke('chat:history', { spaceId, channelId }),
  sendChat: async (request) => invoke('chat:send', { ...request, file: await outgoingFile(request.file) }),
  linkPreview: (url) => invoke('link:preview', url),
  openExternal: (url) => invoke('app:open-external', url),
  downloadChatFile: (spaceId, channelId, fileId) => invoke('chat:download-file', { spaceId, channelId, fileId }),
  fileSignal: (id, sdp) => invoke('file:signal', { id, sdp }),
  fileRead: (id) => invoke('file:read', id),
  fileChunk: (id, data) => ipcRenderer.send('file:chunk', { id, data }),
  fileFinish: (id) => invoke('file:finish', id),
  fileClose: (id, reason) => invoke('file:close', { id, reason }),
  chatImage: (spaceId, channelId, imageId) => invoke('chat:image', { spaceId, channelId, imageId }),
  saveChatFile: (spaceId, channelId, fileId) => invoke('chat:save-file', { spaceId, channelId, fileId }),
  saveChatImage: (spaceId, channelId, imageId) => invoke('chat:save-image', { spaceId, channelId, imageId }),

  voiceJoin: (request) => invoke('voice:join', request),
  voiceUpdate: (changes) => invoke('voice:update', changes),
  voiceLeave: () => invoke('voice:leave'),
  rtcSend: (spaceId, to, payload) => invoke('rtc:send', { spaceId, to, payload }),

  goLive: (request) => invoke('stream:go-live', request),
  stopLive: () => invoke('stream:stop'),
  streamSwitchTarget: (target) => invoke('stream:switch-target', target),
  streamSetQuality: (quality) => invoke('stream:set-quality', quality),
  streamSetAudio: (audio) => invoke('stream:set-audio', audio),
  watchStream: (sharerId) => invoke('stream:watch', sharerId),
  unwatchStream: (sharerId) => invoke('stream:unwatch', sharerId),
  streamFullscreen: (sharerId, enabled) => invoke('stream:fullscreen', { sharerId, enabled }),
  streamVolume: (sharerId, volume) => invoke('stream:volume', { sharerId, volume }),
  streamAnswer: (sharerId, sdp) => invoke('stream:answer', { sharerId, sdp }),
  streamEmbeddedState: (sharerId, state) => invoke('stream:embedded-state', { sharerId, state }),
  publishStreamThumb: (image) => invoke('stream:thumb', image),
  streamPopout: (sharerId) => invoke('stream:popout', sharerId),
  streamPopin: (sharerId) => invoke('stream:popin', sharerId),
  streamsState: () => invoke('streams:state'),

  listSounds: () => invoke('sounds:list'),
  addSound: (sound) => invoke('sounds:add', sound),
  renameSound: (id, name) => invoke('sounds:rename', { id, name }),
  removeSound: (id) => invoke('sounds:remove', id),
  soundData: (id) => invoke('sounds:data', id),

  signalingStatus: () => invoke('signaling:status'),
  copyText: (text) => invoke('clipboard:write', text),
  readClipboard: () => invoke('clipboard:read'),

  onEvent: (listener) => subscribe('telinha:event', listener),
  onLog: (listener) => subscribe('telinha:log', listener),
  onExit: (listener) => subscribe('telinha:exit', listener),
  onSessionStarted: (listener) => subscribe('session:started', listener),
  onContactSaved: (listener) => subscribe('contact:saved', listener),
  onContacts: (listener) => subscribe('contacts:changed', listener),
  onServers: (listener) => subscribe('servers:changed', listener),
  onProfile: (listener) => subscribe('profile:changed', listener),
  onPresence: (listener) => subscribe('presence:changed', listener),
  onVoice: (listener) => subscribe('voice:changed', listener),
  onRtc: (listener) => subscribe('rtc:message', listener),
  onIncomingCall: (listener) => subscribe('call:incoming', listener),
  onChatMessage: (listener) => subscribe('chat:message', listener),
  onChatUpdate: (listener) => subscribe('chat:update', listener),
  onUploadProgress: (listener) => subscribe('chat:upload-progress', listener),
  onFileProgress: (listener) => subscribe('file:progress', listener),
  onFileStatus: (listener) => subscribe('file:status', listener),
  onFileStart: (listener) => subscribe('file:start', listener),
  onFileSignal: (listener) => subscribe('file:signal', listener),
  onFileStop: (listener) => subscribe('file:stop', listener),
  onSignalingStatus: (listener) => subscribe('signaling:status', listener),
  onStreams: (listener) => subscribe('streams:changed', listener),
  onStreamEvent: (listener) => subscribe('stream:event', listener),
  onStreamLog: (listener) => subscribe('stream:log', listener),
  onOutgoingEnded: (listener) => subscribe('stream:outgoing-ended', listener),
  onIncomingEnded: (listener) => subscribe('stream:incoming-ended', listener),
  onStreamOffer: (listener) => subscribe('stream:offer', listener),
  onStreamThumb: (listener) => subscribe('stream:thumb', listener),
});
