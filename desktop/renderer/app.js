'use strict';

const api = window.telinha;

const DM_ROOM = 'dm';
const SHARE_STEPS = ['Preparando', 'Mande o convite', 'Cole a resposta', 'Conectando'];
const WATCH_STEPS = ['Preparando', 'Cole o convite', 'Mande a resposta', 'Conectando'];
const MANUAL_ACTIVE = new Set(['starting', 'live']);
const AUDIO_NAMES = {
  system: 'som do computador', process: 'só o som do programa', device: 'som da placa de captura', none: 'sem som',
};
const TOAST_MS = 6000;

const ICONS = {
  home: '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="5" width="18" height="11" rx="2" fill="none" stroke="currentColor" stroke-width="2"/><path d="M9 20h6M12 16v4" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  hash: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M10 3 8 21M16 3l-2 18M4 8.5h17M3 15.5h17" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  speaker: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 9v6h4l5 4V5L8 9zM16 8.5a5 5 0 0 1 0 7M18.5 6a8.5 8.5 0 0 1 0 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  phone: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6.6 3h3l1.6 4.4-2 1.5a11 11 0 0 0 5.9 5.9l1.5-2 4.4 1.6v3a2 2 0 0 1-2.2 2A17 17 0 0 1 4.6 5.2 2 2 0 0 1 6.6 3z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>',
  screen: '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="12" rx="2" fill="none" stroke="currentColor" stroke-width="2"/><path d="M12 13V7M9 10l3-3 3 3M8 20h8" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  camera: '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="6" width="13" height="12" rx="2" fill="none" stroke="currentColor" stroke-width="2"/><path d="m16 10 5-3v10l-5-3z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>',
  mic: '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="9" y="3" width="6" height="11" rx="3" fill="none" stroke="currentColor" stroke-width="2"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  micOff: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3a3 3 0 0 0-3 3v5a3 3 0 0 0 5.2 2M15 10V6a3 3 0 0 0-5.6-1.5M5 11a7 7 0 0 0 11.5 5.4M19 11a7 7 0 0 1-.6 2.8M12 18v3M4 4l16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  headphones: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 15v-3a8 8 0 0 1 16 0v3" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><path d="M4 15h3v5H5a1 1 0 0 1-1-1zM17 15h3v4a1 1 0 0 1-1 1h-2z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>',
  deaf: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 15v-3a8 8 0 0 1 13.7-5.6M20 12v3M4 15h3v5H5a1 1 0 0 1-1-1zM17 15h3v4a1 1 0 0 1-1 1h-2zM4 4l16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  gear: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="3" fill="none" stroke="currentColor" stroke-width="2"/><path d="M12 2v3M12 19v3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M2 12h3M19 12h3M4.9 19.1 7 17M17 7l2.1-2.1" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  more: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="5" cy="12" r="2" fill="currentColor"/><circle cx="12" cy="12" r="2" fill="currentColor"/><circle cx="19" cy="12" r="2" fill="currentColor"/></svg>',
  invite: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="10" cy="8" r="4" fill="none" stroke="currentColor" stroke-width="2"/><path d="M3 20c.8-4 3.6-6 7-6s6.2 2 7 6M19 8v6M16 11h6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  members: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="9" cy="8" r="3.5" fill="none" stroke="currentColor" stroke-width="2"/><path d="M2.5 20c.7-3.6 3.3-5.5 6.5-5.5s5.8 1.9 6.5 5.5M16 4.5a3.5 3.5 0 0 1 0 7M18 14.8c2 .7 3.1 2.4 3.5 5.2" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
};

const byId = (id) => document.getElementById(id);

const el = {
  railHome: byId('rail-home'),
  railServers: byId('rail-servers'),
  railAdd: byId('rail-add'),
  newConnection: byId('new-connection'),
  serverHeader: byId('server-header'),
  serverTitle: byId('server-title'),
  serverMenu: byId('server-menu'),
  homeNav: byId('home-nav'),
  signalingStatus: byId('signaling-status'),
  contactList: byId('contact-list'),
  contactsEmpty: byId('contacts-empty'),
  serverNav: byId('server-nav'),
  textChannels: byId('text-channels'),
  voiceChannels: byId('voice-channels'),
  channelsEmpty: byId('channels-empty'),
  callPanel: byId('call-panel'),
  callStatus: byId('call-status'),
  callSignal: byId('call-signal'),
  callTitle: byId('call-title'),
  callLeave: byId('call-leave'),
  callOpen: byId('call-open'),
  callCamera: byId('call-camera'),
  callShare: byId('call-share'),
  callSettings: byId('call-settings'),
  callFullscreen: byId('call-fullscreen'),
  callDebug: byId('call-debug'),
  userButton: byId('user-button'),
  userInitial: byId('user-initial'),
  userPresence: byId('user-presence'),
  userName: byId('user-name'),
  userStatus: byId('user-status'),
  userMic: byId('user-mic'),
  userDeaf: byId('user-deaf'),
  userSettings: byId('user-settings'),
  headerIcon: byId('header-icon'),
  headerTitle: byId('header-title'),
  headerSubtitle: byId('header-subtitle'),
  headerActions: byId('header-actions'),
  viewHome: byId('view-home'),
  chooseShare: byId('choose-share'),
  chooseWatch: byId('choose-watch'),
  chooseServer: byId('choose-server'),
  viewSession: byId('view-session'),
  sessionTitle: byId('session-title'),
  sessionStatus: byId('session-status'),
  steps: byId('steps'),
  panelCodeOut: byId('panel-code-out'),
  codeOutTitle: byId('code-out-title'),
  codeOutHint: byId('code-out-hint'),
  codeOut: byId('code-out'),
  copyCode: byId('copy-code'),
  copyFeedback: byId('copy-feedback'),
  panelCodeIn: byId('panel-code-in'),
  codeInTitle: byId('code-in-title'),
  codeInHint: byId('code-in-hint'),
  codeIn: byId('code-in'),
  codeInError: byId('code-in-error'),
  pasteCode: byId('paste-code'),
  submitCode: byId('submit-code'),
  stage: byId('stage'),
  stageTitle: byId('stage-title'),
  stageText: byId('stage-text'),
  stageFeedback: byId('stage-feedback'),
  stageSaved: byId('stage-saved'),
  stageSavedText: byId('stage-saved-text'),
  stageOpenChat: byId('stage-open-chat'),
  stageRename: byId('stage-rename'),
  stageSwitch: byId('stage-switch'),
  stageSettings: byId('stage-settings'),
  stageFullscreen: byId('stage-fullscreen'),
  stageDebug: byId('stage-debug'),
  stageStop: byId('stage-stop'),
  stageDismiss: byId('stage-dismiss'),
  viewEmpty: byId('view-empty'),
  emptyTitle: byId('empty-title'),
  emptyText: byId('empty-text'),
  viewChat: byId('view-chat'),
  roomStage: byId('room-stage'),
  roomTiles: byId('room-tiles'),
  roomFeedback: byId('room-feedback'),
  roomMic: byId('room-mic'),
  roomCamera: byId('room-camera'),
  roomShare: byId('room-share'),
  roomSwitch: byId('room-switch'),
  roomSettings: byId('room-settings'),
  roomLeave: byId('room-leave'),
  roomLobby: byId('room-lobby'),
  lobbyTitle: byId('lobby-title'),
  lobbyText: byId('lobby-text'),
  lobbyPeople: byId('lobby-people'),
  lobbyJoin: byId('lobby-join'),
  members: byId('members'),
  membersList: byId('members-list'),
  callDialog: byId('call-dialog'),
  callDialogTitle: byId('call-dialog-title'),
  callDialogText: byId('call-dialog-text'),
  callDialogAccept: byId('call-dialog-accept'),
  callDialogDecline: byId('call-dialog-decline'),
  promptDialog: byId('prompt-dialog'),
  promptTitle: byId('prompt-title'),
  promptInput: byId('prompt-input'),
  promptDanger: byId('prompt-danger'),
  promptCancel: byId('prompt-cancel'),
  promptSave: byId('prompt-save'),
  serverDialog: byId('server-dialog'),
  serverDialogClose: byId('server-dialog-close'),
  serverCreateName: byId('server-create-name'),
  serverCreate: byId('server-create'),
  serverJoinCode: byId('server-join-code'),
  serverJoinPaste: byId('server-join-paste'),
  serverJoin: byId('server-join'),
  serverDialogError: byId('server-dialog-error'),
  inviteDialog: byId('invite-dialog'),
  inviteTitle: byId('invite-title'),
  inviteClose: byId('invite-close'),
  inviteCode: byId('invite-code'),
  inviteCopy: byId('invite-copy'),
  inviteFeedback: byId('invite-feedback'),
  inviteContactsTitle: byId('invite-contacts-title'),
  inviteContacts: byId('invite-contacts'),
  channelDialog: byId('channel-dialog'),
  channelTitle: byId('channel-title'),
  channelClose: byId('channel-close'),
  channelKind: byId('channel-kind'),
  channelName: byId('channel-name'),
  channelRemove: byId('channel-remove'),
  channelCancel: byId('channel-cancel'),
  channelSave: byId('channel-save'),
  settingsDialog: byId('settings-dialog'),
  settingsClose: byId('settings-close'),
  settingsName: byId('settings-name'),
  settingsNameSave: byId('settings-name-save'),
  settingsMic: byId('settings-mic'),
  settingsSpeaker: byId('settings-speaker'),
  settingsMeterFill: byId('settings-meter-fill'),
  settingsMicStatus: byId('settings-mic-status'),
  settingsPreview: byId('settings-preview'),
  settingsPreviewEmpty: byId('settings-preview-empty'),
  settingsCamera: byId('settings-camera'),
  settingsPreviewToggle: byId('settings-preview-toggle'),
  settingsBlur: byId('settings-blur'),
  settingsCameraStatus: byId('settings-camera-status'),
  contextMenu: byId('context-menu'),
  tileViewer: byId('tile-viewer'),
  tileViewerVideo: byId('tile-viewer-video'),
  tileViewerName: byId('tile-viewer-name'),
  roomResizer: byId('room-resizer'),
  callSounds: byId('call-sounds'),
  voicePopover: byId('voice-popover'),
  popoverConnection: byId('popover-connection'),
  popoverPrivacy: byId('popover-privacy'),
  popoverGraph: byId('popover-graph'),
  popoverTimes: byId('popover-times'),
  popoverPlace: byId('popover-place'),
  popoverAverage: byId('popover-average'),
  popoverLast: byId('popover-last'),
  popoverLoss: byId('popover-loss'),
  popoverDebug: byId('popover-debug'),
  popoverCopy: byId('popover-copy'),
  soundboard: byId('soundboard'),
  soundboardSearch: byId('soundboard-search'),
  soundboardVolume: byId('soundboard-volume'),
  soundboardGrid: byId('soundboard-grid'),
  soundboardHint: byId('soundboard-hint'),
  soundboardFile: byId('soundboard-file'),
  userAvatarImage: byId('user-avatar-image'),
  settingsAvatar: byId('settings-avatar'),
  settingsAvatarChange: byId('settings-avatar-change'),
  settingsAvatarRemove: byId('settings-avatar-remove'),
  toast: byId('toast'),
};

const state = {
  profile: { name: 'Você', memberId: '', instanceId: '' },
  signaling: { connected: false },
  contacts: [],
  servers: [],
  presence: new Map(),
  unread: new Map(),
  nav: { space: 'home', view: 'home', id: null },
  lastChannel: new Map(),
  showMembers: true,
  voice: null,
  streams: { outgoing: null, incoming: [] },
  streamThumbs: new Map(),
  outgoing: null,
  pendingQuality: null,
  streamStats: new Map(),
  roomFeedback: '',
  lobbyRoom: null,
  manual: null,
  manualChoice: null,
  lastShare: null,
  incomingCall: null,
  channelEdit: null,
  prompt: null,
  invite: null,
  previewing: false,
  focusTile: null,
  stageHeight: null,
  volumeApplied: new Set(),
  voiceSince: 0,
  sounds: [],
  avatars: new Map(),
  contactMembers: new Map(),
};

let toastTimer = null;

/* utilidades */

function cleanError(error) {
  const message = error && error.message ? error.message : String(error);
  return message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '');
}

function capitalize(text) {
  return `${text.charAt(0).toUpperCase()}${text.slice(1)}`;
}

function initial(name) {
  return String(name || '?').charAt(0).toUpperCase();
}

function findContact(id) {
  return state.contacts.find((contact) => contact.id === id) ?? null;
}

function findServer(id) {
  return state.servers.find((server) => server.id === id) ?? null;
}

function findChannel(server, channelId) {
  return server ? server.channels.find((channel) => channel.id === channelId) ?? null : null;
}

function peersOf(spaceId) {
  return state.presence.get(spaceId) ?? [];
}

function conversationKey(spaceId, channelId) {
  return channelId ? `${spaceId}-${channelId}` : spaceId;
}

function currentServer() {
  return state.nav.space === 'home' ? null : findServer(state.nav.space);
}

function outgoingLive() {
  return Boolean(state.streams.outgoing && state.streams.outgoing.state === 'live');
}

function processPidOf(target) {
  return target && target.kind === 'window' ? target.pid : 0;
}

function streamAudioState(target, audio) {
  return { audio, processPid: processPidOf(target), deviceShare: Boolean(target && target.kind === 'device') };
}

function toast(text) {
  el.toast.textContent = text;
  el.toast.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    el.toast.hidden = true;
  }, TOAST_MS);
}

function make(tag, className, text) {
  const node = document.createElement(tag);
  if (className) {
    node.className = className;
  }
  if (text !== undefined) {
    node.textContent = text;
  }
  return node;
}

function iconNode(name, title) {
  const node = make('span', 'person-icon');
  node.innerHTML = ICONS[name];
  if (title) {
    node.title = title;
  }
  return node;
}

function avatarUrlFor(memberId) {
  if (!memberId) {
    return null;
  }
  if (memberId === state.profile.memberId) {
    return state.profile.avatar || null;
  }
  return state.avatars.get(memberId) || null;
}

function avatarNode(name, online, size = '', memberId = null) {
  const avatar = make('span', size ? `avatar ${size}` : 'avatar', initial(name));
  avatar.style.background = Call.avatarColor(name);
  const url = avatarUrlFor(memberId);
  if (url) {
    avatar.textContent = '';
    avatar.classList.add('has-image');
    const image = make('img');
    image.src = url;
    image.alt = '';
    avatar.append(image);
  }
  if (online !== null) {
    avatar.append(make('span', online ? 'presence online' : 'presence'));
  }
  return avatar;
}

function unreadBadge(count) {
  return make('span', 'unread-badge', count > 99 ? '99+' : String(count));
}

function headerButton(iconName, label, disabled, onClick) {
  const button = make('button', 'icon-button');
  button.type = 'button';
  button.title = label;
  button.setAttribute('aria-label', label);
  button.innerHTML = ICONS[iconName];
  button.disabled = disabled;
  button.addEventListener('click', onClick);
  return button;
}

function actionButton(label, variant, onClick) {
  const button = make('button', variant ? `button ${variant}` : 'button', label);
  button.type = 'button';
  button.addEventListener('click', (event) => {
    event.stopPropagation();
    onClick();
  });
  return button;
}

function finite(value, digits, suffix = '') {
  return Number.isFinite(value) ? `${value.toFixed(digits)}${suffix}` : '—';
}

function formatCount(value) {
  return Number.isFinite(value) ? Number(value).toLocaleString('pt-BR') : '—';
}

function shareRequest({ selection, audio, audioDevice }) {
  const request = {
    targetKind: selection.kind,
    targetHandle: selection.handle,
    targetIndex: selection.index,
    audio,
  };
  if (audio === 'process') {
    request.audioPid = selection.pid;
  }
  if (audio === 'device') {
    request.audioDevice = audioDevice;
  }
  return request;
}

function audioFromChoice(choice) {
  return {
    scope: choice.audio,
    pid: choice.audio === 'process' ? choice.selection.pid : 0,
    device: choice.audio === 'device' ? choice.audioDevice : '',
  };
}

/* nomes e presenca */

function nameForPeer(spaceId, peer) {
  const contact = findContact(spaceId);
  if (contact) {
    return contact.name;
  }
  const server = findServer(spaceId);
  const member = server && peer.memberId
    ? server.members.find((item) => item.memberId === peer.memberId)
    : null;
  return (member && member.name) || peer.name || 'Alguém';
}

function voicePeople(spaceId, roomId) {
  const people = peersOf(spaceId)
    .filter((peer) => peer.voice && peer.voice.roomId === roomId)
    .map((peer) => ({
      id: peer.id,
      name: nameForPeer(spaceId, peer),
      mic: peer.voice.mic,
      deaf: peer.voice.deaf,
      camera: peer.voice.camera,
      live: peer.voice.live,
      self: false,
      memberId: peer.memberId,
    }));
  if (state.voice && state.voice.spaceId === spaceId && state.voice.roomId === roomId) {
    const call = Call.state();
    people.unshift({
      id: state.profile.instanceId,
      name: state.profile.name,
      mic: call.micLive,
      deaf: call.deaf,
      camera: call.cameraOn,
      live: outgoingLive(),
      self: true,
      memberId: state.profile.memberId,
    });
  }
  return people;
}

function contactStatus(contact) {
  const peers = peersOf(contact.id);
  const inCall = peers.filter((peer) => peer.voice && peer.voice.roomId === DM_ROOM);
  if (inCall.some((peer) => peer.voice.live)) {
    return 'transmitindo';
  }
  if (inCall.length > 0) {
    return 'em chamada';
  }
  return peers.length > 0 || contact.online ? 'online' : 'offline';
}

function placeName(voice) {
  const contact = findContact(voice.spaceId);
  if (contact) {
    return `Chamada com ${contact.name}`;
  }
  const server = findServer(voice.spaceId);
  const channel = findChannel(server, voice.roomId);
  return server ? `${channel ? channel.name : 'Sala'} / ${server.name}` : 'Chamada';
}

/* conversa */

function chatHint(contact) {
  if (!state.signaling.connected) {
    return 'Sem conexão com o servidor de contatos. As mensagens não vão chegar agora.';
  }
  if (contactStatus(contact) === 'offline') {
    return `${contact.name} está offline. A mensagem só chega com o Telinha aberto do outro lado.`;
  }
  return 'Enter envia, Shift+Enter pula linha.';
}

function conversationFor(nav) {
  if (nav.view !== 'chat' || !nav.id) {
    return null;
  }
  if (nav.space === 'home') {
    const contact = findContact(nav.id);
    if (!contact) {
      return null;
    }
    return {
      key: contact.id,
      spaceId: contact.id,
      channelId: null,
      title: contact.name,
      introText: 'Este é o começo da conversa. As mensagens ficam guardadas só neste computador e só chegam quando a outra pessoa está com o Telinha aberto.',
      placeholder: `Conversar com @${contact.name}`,
      authorFor: () => contact.name,
      avatarFor: (message) => (message.mine
        ? state.profile.avatar
        : avatarUrlFor(state.contactMembers.get(contact.id))),
      hint: () => chatHint(contact),
      onInvite: (code) => joinServer(code),
    };
  }

  const server = findServer(nav.space);
  const channel = findChannel(server, nav.id);
  if (!channel) {
    return null;
  }
  const names = new Map(server.members.map((member) => [member.memberId, member.name]));
  const text = channel.kind === 'text';
  return {
    key: conversationKey(server.id, channel.id),
    spaceId: server.id,
    channelId: channel.id,
    title: channel.name,
    introIcon: text ? '#' : '♪',
    introTitle: text ? `Bem-vindo a #${channel.name}` : `Chat de ${channel.name}`,
    introText: text
      ? `Este é o começo do canal #${channel.name} em ${server.name}. Cada computador guarda o próprio histórico, e quem está offline não recebe.`
      : 'Converse aqui com quem está na sala. As mensagens ficam guardadas só neste computador.',
    placeholder: text ? `Conversar em #${channel.name}` : `Conversar em ${channel.name}`,
    authorFor: (message) => names.get(message.memberId) ?? message.author,
    avatarFor: (message) => (message.mine ? state.profile.avatar : avatarUrlFor(message.memberId)),
    hint: () => (state.signaling.connected
      ? ''
      : 'Sem conexão com o servidor de contatos. As mensagens não vão chegar agora.'),
    onInvite: (code) => joinServer(code),
  };
}

function refreshConversation() {
  const conversation = conversationFor(state.nav);
  if (conversation) {
    Chat.refresh(conversation);
  }
}

/* navegacao */

function navigate(nav) {
  state.nav = nav;
  const conversation = conversationFor(nav);
  if (conversation) {
    state.unread.delete(conversation.key);
    Chat.open(conversation);
  } else {
    Chat.close();
  }
  el.serverMenu.hidden = true;
  render();
}

function openHome() {
  navigate({ space: 'home', view: 'home', id: null });
}

function openManualView() {
  navigate({ space: 'home', view: 'session', id: null });
}

function openContact(contactId) {
  if (findContact(contactId)) {
    navigate({ space: 'home', view: 'chat', id: contactId });
  }
}

function openChannel(serverId, channelId) {
  if (channelId) {
    state.lastChannel.set(serverId, channelId);
  }
  navigate({ space: serverId, view: 'chat', id: channelId });
}

function openServer(serverId) {
  const server = findServer(serverId);
  if (!server) {
    return;
  }
  const channel = findChannel(server, state.lastChannel.get(serverId))
    ?? server.channels.find((item) => item.kind === 'text')
    ?? server.channels[0]
    ?? null;
  openChannel(serverId, channel ? channel.id : null);
}

function openVoicePlace() {
  const { voice } = state;
  if (!voice) {
    return;
  }
  if (findContact(voice.spaceId)) {
    openContact(voice.spaceId);
  } else {
    openChannel(voice.spaceId, voice.roomId);
  }
}

/* desenho */

function render() {
  renderRail();
  renderSidebar();
  renderCallPanel();
  renderUserPanel();
  renderHeader();
  renderViews();
  renderMembers();
}

function serverInitials(name) {
  return name.split(/\s+/).filter(Boolean).slice(0, 2).map((word) => word.charAt(0).toUpperCase()).join('') || '?';
}

function serverUnread(server) {
  return server.channels.reduce((sum, channel) => sum + (state.unread.get(conversationKey(server.id, channel.id)) ?? 0), 0);
}

function renderRail() {
  el.railHome.classList.toggle('active', state.nav.space === 'home');
  el.railHome.classList.toggle('unread', state.contacts.some((contact) => (state.unread.get(contact.id) ?? 0) > 0));
  el.railServers.replaceChildren(...state.servers.map((server) => {
    const button = make('button', 'rail-button rail-server', serverInitials(server.name));
    button.type = 'button';
    button.title = server.name;
    button.setAttribute('aria-label', server.name);
    button.classList.toggle('active', state.nav.space === server.id);
    button.classList.toggle('unread', serverUnread(server) > 0);
    button.classList.toggle('in-voice', Boolean(state.voice && state.voice.spaceId === server.id));
    button.addEventListener('click', () => openServer(server.id));
    return button;
  }));
}

function renderSidebar() {
  const server = currentServer();
  el.newConnection.hidden = Boolean(server);
  el.serverHeader.hidden = !server;
  el.homeNav.hidden = Boolean(server);
  el.serverNav.hidden = !server;
  if (!server) {
    el.serverMenu.hidden = true;
  }

  const connected = Boolean(state.signaling.connected);
  el.signalingStatus.className = `signal ${connected ? 'online' : 'offline'}`;
  el.signalingStatus.title = connected ? 'Conectado ao servidor de contatos' : 'Sem conexão com o servidor de contatos';

  if (server) {
    el.serverTitle.textContent = server.name;
    renderChannels(server);
  } else {
    renderContacts();
  }
}

function renderContacts() {
  el.contactsEmpty.hidden = state.contacts.length > 0;
  el.contactList.replaceChildren(...state.contacts.map((contact) => {
    const unread = state.unread.get(contact.id) ?? 0;
    const button = make('button', 'contact-button');
    button.type = 'button';
    button.classList.toggle('active', state.nav.space === 'home' && state.nav.view === 'chat' && state.nav.id === contact.id);
    button.classList.toggle('unread', unread > 0);

    const status = contactStatus(contact);
    const info = make('span', 'nav-info');
    const name = make('span', 'nav-name', contact.name);
    name.title = contact.name;
    info.append(name, make('span', 'nav-sub', status));
    button.append(avatarNode(contact.name, status !== 'offline', '', state.contactMembers.get(contact.id)), info);
    if (unread > 0) {
      button.append(unreadBadge(unread));
    }
    button.addEventListener('click', () => openContact(contact.id));
    button.addEventListener('contextmenu', (event) => {
      event.preventDefault();
      openPersonMenu(event, personFromContact(contact));
    });

    const item = make('li');
    item.append(button);
    return item;
  }));
}

function personRow(person, spaceId, roomId) {
  const item = make('li', 'voice-person');
  item.dataset.voiceId = person.id;
  item.addEventListener('contextmenu', (event) => {
    event.preventDefault();
    openPersonMenu(event, personFromVoice(spaceId, roomId, person));
  });
  item.append(avatarNode(person.name, null, 'avatar-small', person.memberId),
    make('span', 'voice-person-name', person.name));
  if (person.live) {
    item.append(make('span', 'live-badge small', 'AO VIVO'));
  }
  if (person.camera) {
    item.append(iconNode('camera', 'Câmera ligada'));
  }
  if (person.deaf) {
    item.append(iconNode('deaf', 'Áudio desativado'));
  } else if (!person.mic) {
    item.append(iconNode('micOff', 'Microfone silenciado'));
  }
  return item;
}

function channelRow(server, channel) {
  const item = make('li', 'channel-row');
  const key = conversationKey(server.id, channel.id);
  const unread = state.unread.get(key) ?? 0;

  const button = make('button', 'channel-button');
  button.type = 'button';
  button.classList.toggle('active', state.nav.space === server.id && state.nav.id === channel.id);
  button.classList.toggle('unread', unread > 0);
  button.classList.toggle('joined', Boolean(state.voice && state.voice.spaceId === server.id
    && state.voice.roomId === channel.id));
  const glyph = make('span', 'channel-glyph');
  glyph.innerHTML = channel.kind === 'text' ? ICONS.hash : ICONS.speaker;
  button.append(glyph, make('span', 'nav-name', channel.name));
  if (channel.kind === 'voice' && state.voice && state.voice.spaceId === server.id
    && state.voice.roomId === channel.id) {
    button.append(make('span', 'voice-timer', formatElapsed(Date.now() - state.voiceSince)));
  }
  if (unread > 0) {
    button.append(unreadBadge(unread));
  }
  button.addEventListener('click', () => {
    openChannel(server.id, channel.id);
    if (channel.kind === 'voice' && !Call.inRoom(server.id, channel.id)) {
      joinRoom(server.id, channel.id);
    }
  });

  const gear = make('button', 'channel-gear');
  gear.type = 'button';
  gear.title = 'Editar canal';
  gear.setAttribute('aria-label', `Editar ${channel.name}`);
  gear.innerHTML = ICONS.gear;
  gear.addEventListener('click', () => openChannelDialog(server.id, channel));

  item.append(button, gear);
  return item;
}

function renderChannels(server) {
  const text = server.channels.filter((channel) => channel.kind === 'text');
  const voice = server.channels.filter((channel) => channel.kind === 'voice');
  el.channelsEmpty.hidden = server.channels.length > 0;
  el.textChannels.replaceChildren(...text.map((channel) => channelRow(server, channel)));
  el.voiceChannels.replaceChildren(...voice.map((channel) => {
    const item = channelRow(server, channel);
    const people = voicePeople(server.id, channel.id);
    if (people.length > 0) {
      const list = make('ul', 'voice-people');
      list.append(...people.map((person) => personRow(person, server.id, channel.id)));
      item.append(list);
    }
    return item;
  }));
}

function renderCallPanel() {
  const { voice } = state;
  const manual = activeManual();
  el.callPanel.hidden = !voice && !manual;

  if (voice) {
    const call = Call.state();
    el.callTitle.textContent = 'Voz conectada';
    el.callTitle.className = 'call-title';
    el.callSignal.setAttribute('class', `call-signal ${Debug.quality('call')}`);
    el.callOpen.textContent = placeName(voice);
    el.callCamera.hidden = false;
    el.callSounds.hidden = false;
    el.callCamera.classList.toggle('active', call.cameraOn);
    el.callCamera.title = call.cameraOn ? 'Desligar câmera' : 'Ligar câmera';
    el.callShare.hidden = false;
    el.callShare.disabled = false;
    el.callShare.classList.toggle('live', Boolean(state.streams.outgoing));
    el.callShare.title = state.streams.outgoing ? 'Parar de transmitir' : 'Transmitir tela';
    el.callSettings.hidden = !outgoingLive();
    el.callSettings.disabled = false;
    el.callFullscreen.hidden = true;
    el.callLeave.title = 'Desconectar';
    el.callLeave.disabled = call.busy;
    return;
  }
  if (!manual) {
    return;
  }

  const live = manual.phase === 'live';
  const share = manual.role === 'share';
  el.callTitle.textContent = live ? (share ? 'Transmitindo' : 'Assistindo') : 'Conectando…';
  el.callTitle.className = live ? 'call-title' : 'call-title waiting';
  el.callSignal.setAttribute('class', `call-signal ${live ? Debug.quality('manual') : 'unknown'}`);
  el.callOpen.textContent = 'Conexão com código';
  el.callCamera.hidden = true;
  el.callSounds.hidden = true;
  el.callShare.hidden = !share;
  el.callShare.disabled = !live;
  el.callShare.classList.remove('live');
  el.callShare.title = 'Trocar tela';
  el.callSettings.hidden = !share;
  el.callSettings.disabled = !live;
  el.callFullscreen.hidden = share;
  el.callFullscreen.disabled = !live;
  el.callFullscreen.classList.toggle('active', manual.fullscreen);
  el.callFullscreen.title = manual.fullscreen ? 'Sair da tela cheia' : 'Tela cheia';
  el.callLeave.title = 'Encerrar';
  el.callLeave.disabled = manual.stopping;
}

function renderUserPanel() {
  const connected = Boolean(state.signaling.connected);
  el.userName.textContent = state.profile.name;
  el.userInitial.textContent = initial(state.profile.name);
  el.userInitial.parentElement.style.background = Call.avatarColor(state.profile.name);
  el.userInitial.hidden = Boolean(state.profile.avatar);
  el.userAvatarImage.hidden = !state.profile.avatar;
  if (state.profile.avatar) {
    el.userAvatarImage.src = state.profile.avatar;
  }
  el.userPresence.className = connected ? 'presence online' : 'presence';
  el.userStatus.textContent = connected ? 'online' : 'sem servidor de contatos';

  const call = Call.state();
  const micOff = !call.micOn || call.deaf;
  el.userMic.innerHTML = micOff ? ICONS.micOff : ICONS.mic;
  el.userMic.classList.toggle('off', micOff);
  el.userMic.title = micOff ? 'Ativar microfone' : 'Silenciar microfone';
  el.userDeaf.innerHTML = call.deaf ? ICONS.deaf : ICONS.headphones;
  el.userDeaf.classList.toggle('off', call.deaf);
  el.userDeaf.title = call.deaf ? 'Ativar áudio' : 'Desativar áudio';
}

function renderHeader() {
  const { nav } = state;
  const actions = [];
  const call = Call.state();

  if (nav.space === 'home') {
    if (nav.view === 'chat') {
      const contact = findContact(nav.id);
      el.headerIcon.textContent = '@';
      el.headerTitle.textContent = contact ? contact.name : 'Contato';
      el.headerSubtitle.textContent = contact ? contactStatus(contact) : '';
      if (contact) {
        const inThisCall = Boolean(state.voice && state.voice.spaceId === contact.id);
        actions.push(
          headerButton('phone', inThisCall ? 'Você está nessa chamada' : `Ligar para ${contact.name}`,
            inThisCall || call.busy, () => startCall(contact.id)),
          headerButton('screen', `Transmitir sua tela para ${contact.name}`,
            Boolean(state.streams.outgoing) || call.busy, () => startCallAndShare(contact.id)),
          headerButton('more', 'Renomear ou remover', false, () => openContactMenu(contact)),
        );
      }
    } else {
      el.headerIcon.innerHTML = ICONS.home;
      el.headerTitle.textContent = nav.view === 'session' ? 'Conexão com código' : 'Início';
      el.headerSubtitle.textContent = '';
    }
  } else {
    const server = currentServer();
    const channel = findChannel(server, nav.id);
    el.headerIcon.innerHTML = channel && channel.kind === 'voice' ? ICONS.speaker : ICONS.hash;
    el.headerTitle.textContent = channel ? channel.name : (server ? server.name : '');
    el.headerSubtitle.textContent = channel && channel.kind === 'voice'
      ? `${voicePeople(server.id, channel.id).length} na sala`
      : (server ? server.name : '');
    if (server) {
      actions.push(
        headerButton('invite', 'Convidar pessoas', false, () => openInvite(server.id)),
        headerButton('members', state.showMembers ? 'Esconder membros' : 'Mostrar membros', false, () => {
          state.showMembers = !state.showMembers;
          render();
        }),
      );
    }
  }
  el.headerActions.replaceChildren(...actions);
}

function roomOf(conversation) {
  if (!conversation) {
    return null;
  }
  if (!conversation.channelId) {
    return { spaceId: conversation.spaceId, roomId: DM_ROOM };
  }
  const channel = findChannel(findServer(conversation.spaceId), conversation.channelId);
  return channel && channel.kind === 'voice' ? { spaceId: conversation.spaceId, roomId: channel.id } : null;
}

function renderViews() {
  const { nav } = state;
  const conversation = conversationFor(nav);
  const home = nav.space === 'home';
  el.viewHome.hidden = !(home && nav.view === 'home');
  el.viewSession.hidden = !(home && nav.view === 'session');
  el.viewChat.hidden = !conversation;

  const empty = nav.view === 'chat' && !conversation;
  el.viewEmpty.hidden = !empty;
  if (empty) {
    const server = currentServer();
    el.emptyTitle.textContent = server ? server.name : 'Nada por aqui';
    el.emptyText.textContent = server
      ? 'Os canais aparecem quando alguém que já está no servidor ficar online. Se o servidor é seu, crie um canal no menu com o nome dele.'
      : 'Escolha uma conversa à esquerda.';
  }

  const manualBusy = Boolean(activeManual());
  el.chooseShare.disabled = manualBusy;
  el.chooseWatch.disabled = manualBusy;
  renderRoom(conversation);
  renderManual();
}

function renderRoom(conversation) {
  const room = roomOf(conversation);
  const joined = Boolean(room && state.voice && state.voice.spaceId === room.spaceId
    && state.voice.roomId === room.roomId);
  const people = room ? voicePeople(room.spaceId, room.roomId) : [];
  const lobby = Boolean(room && !joined && (room.roomId !== DM_ROOM || people.length > 0));

  el.viewChat.classList.toggle('with-stage', joined);
  el.viewChat.classList.toggle('voice-channel', Boolean(room && room.roomId !== DM_ROOM));
  el.roomStage.hidden = !joined;
  el.roomLobby.hidden = !lobby;
  if (joined) {
    renderStage();
  }
  if (lobby) {
    renderLobby(room, people);
  }
}

function fillLiveActions(info, container) {
  if (info.self) {
    const { outgoing } = state.streams;
    const viewers = outgoing ? outgoing.viewers.filter((viewer) => viewer.state === 'Connected').length : 0;
    container.append(
      make('span', 'tile-note', viewers === 1 ? '1 assistindo' : `${viewers} assistindo`),
      actionButton('Trocar', '', openOutgoingSwitch),
      actionButton('Parar', 'danger', stopLive),
    );
    return;
  }
  const watching = state.streams.incoming.find((entry) => entry.sharerId === info.id);
  if (!watching) {
    container.append(actionButton('Assistir', 'primary', () => watch(info.id)));
  } else if (watching.state !== 'live') {
    container.append(make('span', 'tile-note', 'Abrindo…'), actionButton('Cancelar', '', () => unwatch(info.id)));
  } else {
    container.append(make('span', 'tile-note', 'Assistindo'), actionButton('Parar de assistir', '', () => unwatch(info.id)));
  }
}

function renderStage() {
  const call = Call.state();
  const { voice } = state;
  Call.renderTiles(el.roomTiles, {
    selfName: state.profile.name,
    selfMemberId: state.profile.memberId,
    avatarFor: (info) => avatarUrlFor(info.memberId),
    outgoingLive: outgoingLive(),
    outgoingName: state.streams.outgoing ? state.streams.outgoing.targetName : '',
    selfPreview: outgoingLive() ? StreamView.video(StreamView.SELF) : null,
    thumbFor: streamThumbFor,
    nameFor: (peer) => nameForPeer(voice.spaceId, peer),
    liveActions: fillLiveActions,
    focusId: state.focusTile,
    onFocus: toggleFocus,
    onContextMenu: (info, event) => openPersonMenu(event, personFromTile(info)),
    onFullscreen: openTileViewer,
    streams: streamTilesInfo(),
    onStreamMenu: (stream, event) => openPersonMenu(event, personFromTile({
      id: stream.sharerId, name: stream.name, self: false,
    })),
    onStreamFullscreen: openStreamViewer,
    onPopout: (stream) => api.streamPopout(stream.sharerId),
    onPopin: (stream) => api.streamPopin(stream.sharerId),
    onStopWatching: (stream) => unwatch(stream.sharerId),
  });
  el.roomFeedback.textContent = state.roomFeedback;
  el.roomMic.innerHTML = call.micLive ? ICONS.mic : ICONS.micOff;
  el.roomMic.classList.toggle('off', !call.micLive);
  el.roomMic.title = call.micLive ? 'Silenciar microfone' : 'Ativar microfone';
  el.roomCamera.classList.toggle('on', call.cameraOn);
  el.roomCamera.title = call.cameraOn ? 'Desligar câmera' : 'Ligar câmera';
  const sharing = Boolean(state.streams.outgoing);
  el.roomShare.classList.toggle('live', sharing);
  el.roomShare.title = sharing ? 'Parar de transmitir' : 'Transmitir tela';
  el.roomSwitch.hidden = !outgoingLive();
  el.roomSettings.hidden = !outgoingLive();
  el.roomLeave.disabled = call.busy;
}

function renderLobby(room, people) {
  const contact = findContact(room.spaceId);
  if (contact) {
    const live = people.some((person) => person.live);
    el.lobbyTitle.textContent = live ? `${contact.name} está transmitindo` : `${contact.name} está em uma chamada`;
    el.lobbyText.textContent = 'Entre para conversar por voz e ver a tela ou a câmera.';
    el.lobbyJoin.textContent = 'Entrar na chamada';
  } else {
    const channel = findChannel(findServer(room.spaceId), room.roomId);
    el.lobbyTitle.textContent = channel ? channel.name : 'Sala';
    if (people.length === 0) {
      el.lobbyText.textContent = 'Ninguém na sala. Entre e chame o pessoal.';
    } else {
      el.lobbyText.textContent = people.length === 1 ? '1 pessoa na sala agora.' : `${people.length} pessoas na sala agora.`;
    }
    el.lobbyJoin.textContent = 'Entrar na sala';
  }
  el.lobbyPeople.replaceChildren(...people.map((person) => {
    const chip = make('span', 'person-chip');
    chip.dataset.voiceId = person.id;
    chip.append(avatarNode(person.name, null, 'avatar-small', person.memberId), make('span', null, person.name));
    if (person.live) {
      chip.append(make('span', 'live-badge small', 'AO VIVO'));
    }
    return chip;
  }));
  el.lobbyJoin.disabled = Call.state().busy;
  state.lobbyRoom = room;
}

function memberRow(server, member, online) {
  const row = make('div', online ? 'member' : 'member offline');
  row.addEventListener('contextmenu', (event) => {
    event.preventDefault();
    openPersonMenu(event, personFromMember(server, member));
  });
  row.append(avatarNode(member.name, online, '', member.memberId));
  const info = make('div', 'member-info');
  info.append(make('span', 'member-name', member.self ? `${member.name} (você)` : member.name));
  if (member.voice) {
    const channel = findChannel(server, member.voice.roomId);
    const place = channel ? channel.name : 'uma sala';
    info.append(make('span', 'member-activity', member.voice.live ? `Transmitindo em ${place}` : `Na sala ${place}`));
  }
  row.append(info);
  return row;
}

function renderMembers() {
  const server = currentServer();
  const show = Boolean(server && state.showMembers && state.nav.view === 'chat');
  el.members.hidden = !show;
  if (!show) {
    return;
  }

  const online = new Map();
  for (const peer of peersOf(server.id)) {
    const key = peer.memberId ?? peer.id;
    if (!online.has(key)) {
      online.set(key, {
        name: nameForPeer(server.id, peer), voice: peer.voice, self: false, peerId: peer.id, memberId: peer.memberId,
      });
    }
  }
  const selfVoice = state.voice && state.voice.spaceId === server.id
    ? { roomId: state.voice.roomId, live: outgoingLive() }
    : null;
  online.set(state.profile.memberId || 'self', {
    name: state.profile.name, voice: selfVoice, self: true, memberId: state.profile.memberId,
  });

  const byName = (a, b) => a.name.localeCompare(b.name, 'pt-BR');
  const onlineList = [...online.values()].sort(byName);
  const offlineList = server.members
    .filter((member) => !online.has(member.memberId))
    .map((member) => ({
      name: member.name, voice: null, self: false, memberId: member.memberId,
    }))
    .sort(byName);

  el.membersList.replaceChildren(
    make('div', 'members-heading', `Online — ${onlineList.length}`),
    ...onlineList.map((member) => memberRow(server, member, true)),
    make('div', 'members-heading', `Offline — ${offlineList.length}`),
    ...offlineList.map((member) => memberRow(server, member, false)),
  );
}

/* chamada e salas */

async function joinRoom(spaceId, roomId) {
  try {
    await Call.join(spaceId, roomId);
    state.roomFeedback = Call.state().micError ?? '';
  } catch (error) {
    toast(cleanError(error));
  }
  render();
}

async function leaveRoom() {
  await Call.leave();
  state.roomFeedback = '';
  render();
}

async function startCall(contactId) {
  openContact(contactId);
  await joinRoom(contactId, DM_ROOM);
}

async function startCallAndShare(contactId) {
  openContact(contactId);
  if (!Call.inRoom(contactId, DM_ROOM)) {
    await joinRoom(contactId, DM_ROOM);
  }
  if (Call.inRoom(contactId, DM_ROOM) && !state.streams.outgoing) {
    openGoLive();
  }
}

async function toggleMic() {
  const call = Call.state();
  await Call.setMicOn(!call.micOn || call.deaf);
  const next = Call.state();
  if (next.micOn && next.micError && state.voice) {
    toast(next.micError);
  }
}

async function toggleDeaf() {
  await Call.setDeaf(!Call.state().deaf);
}

async function toggleCamera() {
  if (!state.voice) {
    return;
  }
  try {
    await Call.setCameraOn(!Call.state().cameraOn);
  } catch (error) {
    toast(cleanError(error));
  }
}

function openGoLive() {
  if (!state.voice) {
    toast('Entre numa chamada ou sala antes de transmitir.');
    return;
  }
  Share.openPicker({
    mode: 'live',
    title: 'Transmitir tela',
    confirmLabel: 'Transmitir',
    settings: true,
    quality: state.lastShare ? state.lastShare.quality : undefined,
    audio: state.lastShare ? state.lastShare.audio : undefined,
    onConfirm: async (choice) => {
      if (!state.voice) {
        return;
      }
      state.lastShare = choice;
      state.pendingQuality = choice.quality;
      state.outgoing = {
        target: choice.selection,
        audio: audioFromChoice(choice),
        audioInputs: choice.audioInputs ?? [],
        quality: choice.quality,
        pendingTarget: null,
      };
      state.roomFeedback = 'Abrindo a transmissão…';
      render();
      try {
        await api.goLive({ share: shareRequest(choice), targetName: choice.selection.name });
      } catch (error) {
        state.outgoing = null;
        state.pendingQuality = null;
        state.roomFeedback = '';
        toast(cleanError(error));
        render();
      }
    },
  });
}

function toggleLive() {
  if (state.streams.outgoing) {
    stopLive();
  } else {
    openGoLive();
  }
}

async function stopLive() {
  await api.stopLive();
}

async function watch(sharerId) {
  try {
    await api.watchStream(sharerId);
    state.focusTile = `stream:${sharerId}`;
  } catch (error) {
    toast(cleanError(error));
  }
}

async function unwatch(sharerId) {
  await api.unwatchStream(sharerId);
}

function openOutgoingSwitch() {
  if (!outgoingLive() || !state.outgoing) {
    return;
  }
  Share.openPicker({
    mode: 'switch',
    title: 'Trocar o que você está mostrando',
    confirmLabel: 'Trocar',
    settings: false,
    onConfirm: async ({ selection, audioInputs }) => {
      const info = state.outgoing;
      if (!info) {
        return;
      }
      info.audioInputs = audioInputs ?? info.audioInputs;
      info.pendingTarget = selection;
      state.roomFeedback = 'Trocando de tela…';
      render();
      if (!(await api.streamSwitchTarget({ kind: selection.kind, handle: selection.handle }))) {
        info.pendingTarget = null;
        toast('Não consegui pedir a troca agora.');
      }
    },
  });
}

function openOutgoingSettings() {
  const info = state.outgoing;
  if (!outgoingLive() || !info) {
    return;
  }
  Share.openStream({
    ...streamAudioState(info.target, info.audio),
    audioInputs: info.audioInputs ?? [],
    quality: info.quality,
    onQuality: async (quality) => {
      info.quality = quality;
      Share.streamFeedback('Aplicando…');
      if (!(await api.streamSetQuality(quality))) {
        Share.streamFeedback('Não consegui mudar a qualidade agora.');
      }
    },
    onAudio: async (scope, pid, device) => {
      Share.streamFeedback('Trocando o som…');
      if (!(await api.streamSetAudio({ scope, pid, device }))) {
        Share.streamFeedback('Não consegui trocar o som agora.');
      }
    },
  });
}

function callPanelShare() {
  if (state.voice) {
    toggleLive();
  } else {
    manualSwitch();
  }
}

function callPanelSettings() {
  if (state.voice) {
    openOutgoingSettings();
  } else {
    manualSettings();
  }
}

function callPanelLeave() {
  if (state.voice) {
    leaveRoom();
  } else {
    stopManual();
  }
}

function callPanelOpen() {
  if (state.voice) {
    openVoicePlace();
  } else {
    openManualView();
  }
}

function callPanelDebug() {
  Debug.open(state.voice ? 'call' : 'manual');
}

/* chamadas recebidas */

function closeIncomingCall() {
  el.callDialog.hidden = true;
  state.incomingCall = null;
}

function handleIncomingCall({ contactId, name, live }) {
  if (state.voice && state.voice.spaceId === contactId) {
    return;
  }
  state.incomingCall = { contactId, name, live };
  el.callDialogTitle.textContent = live ? `${name} está transmitindo a tela para você` : `${name} está chamando você`;
  el.callDialogText.textContent = live
    ? 'Entre na chamada para assistir e conversar.'
    : 'Entre para conversar por voz, ver a câmera ou a tela.';
  el.callDialogAccept.textContent = live ? 'Entrar e assistir' : 'Entrar';
  el.callDialog.hidden = false;
}

async function acceptIncomingCall() {
  const call = state.incomingCall;
  closeIncomingCall();
  if (!call) {
    return;
  }
  openContact(call.contactId);
  await joinRoom(call.contactId, DM_ROOM);
  if (call.live && Call.inRoom(call.contactId, DM_ROOM)) {
    const sharer = peersOf(call.contactId).find((peer) => peer.voice && peer.voice.roomId === DM_ROOM && peer.voice.live);
    if (sharer) {
      watch(sharer.id);
    }
  }
}

/* estatisticas */

function frameRate(source, count) {
  const now = performance.now();
  const previous = state.streamStats.get(source);
  state.streamStats.set(source, { at: now, count });
  if (!previous) {
    return null;
  }
  const seconds = (now - previous.at) / 1000;
  return seconds > 0 ? Math.max(0, (count - previous.count) / seconds) : null;
}

function senderRows(event, fps) {
  const rows = [
    ['Quadros por segundo', finite(fps, 0)],
    ['Resolução', event.width ? `${event.width}×${event.height}` : '—'],
    ['Ping', finite(event.rtt_ms, 0, ' ms')],
    ['Perda de pacotes', finite(event.loss * 100, 2, ' %')],
    ['Taxa de envio', finite(event.bitrate_bps / 1e6, 2, ' Mbps')],
    ['Captura', finite(event.capture_ms, 2, ' ms')],
    ['Encode', finite(event.encode_ms, 2, ' ms')],
    ['Quadros enviados', formatCount(event.frames)],
    ['Keyframes', formatCount(event.keyframes)],
  ];
  if (Number.isFinite(event.viewers)) {
    rows.push(['Assistindo', formatCount(event.viewers)]);
  }
  return rows;
}

function receiverRows(event, fps) {
  return [
    ['Quadros por segundo', finite(fps, 0)],
    ['Ping', finite(event.rtt_ms, 0, ' ms')],
    ['Perda de pacotes', finite(event.loss * 100, 2, ' %')],
    ['Atraso de reprodução', finite(event.delay_ms, 0, ' ms')],
    ['Intervalo entre quadros', finite(event.present_interval_ms, 1, ' ms')],
    ['Quadros exibidos', formatCount(event.presented)],
    ['Quadros recebidos', formatCount(event.frames)],
    ['Descartados', formatCount(event.dropped)],
    ['Lacunas', formatCount(event.gaps)],
    ['Blocos de áudio', formatCount(event.audio_blocks)],
  ];
}

function handleCallStats(summary) {
  if (!state.voice) {
    return;
  }
  const call = Call.state();
  Debug.update('call', {
    label: 'Chamada',
    rtt: summary.rtt,
    loss: summary.loss,
    rows: [
      ['Conectados', `${summary.connected} de ${summary.peers}`],
      ['Ping médio', finite(summary.rtt, 0, ' ms')],
      ['Perda de pacotes', finite(Number.isFinite(summary.loss) ? summary.loss * 100 : NaN, 2, ' %')],
      ['Voz reconstruída', finite(Number.isFinite(summary.concealment) ? summary.concealment * 100 : NaN, 1, ' %')],
      ['Voz esticada', finite(Number.isFinite(summary.stretch) ? summary.stretch * 100 : NaN, 1, ' %')],
      ['Buffer da voz', finite(summary.buffer, 0, ' ms')],
      ['Microfone', call.micLive ? 'ligado' : 'mudo'],
      ['Câmera', call.cameraOn ? 'ligada' : 'desligada'],
    ],
    summary: `${placeName(state.voice)} · voz e câmera direto entre os computadores`,
  });
  renderVoicePopover();
  renderCallPanel();
}

/* descricoes */

function describeQuality(event) {
  const fps = event.max_fps ? `até ${event.max_fps} fps` : 'até 60 fps';
  const bitrate = event.max_bitrate_kbps
    ? `até ${(event.max_bitrate_kbps / 1000).toLocaleString('pt-BR')} Mbps`
    : 'taxa automática';
  return `Qualidade aplicada: ${event.width}×${event.height}, ${fps}, ${bitrate}.`;
}

function describeCommandFailure(event) {
  switch (event.command) {
    case 'switch_target':
      return 'Não consegui trocar para essa tela, continuo mostrando a anterior.';
    case 'set_audio':
      return `Não consegui trocar o som (${event.message}), continuo com o anterior.`;
    default:
      return `Não consegui mudar a qualidade (${event.message}).`;
  }
}

function describeError(event) {
  switch (event.stage) {
    case 'network':
      return 'Não foi possível abrir o caminho direto até a outra máquina. Redirecione as portas UDP 50000 a 50039 no roteador, ou coloquem os computadores na mesma rede do ZeroTier.';
    case 'connection_lost':
      return 'A conexão caiu e não volta sozinha. Comecem de novo.';
    case 'audio':
      return `Som: ${event.message}. Escolha "Som do computador" ou "Sem som" e tente de novo.`;
    case 'prepare':
      return `Não consegui preparar o Telinha nesta máquina (${event.status}: ${event.message}).`;
    default:
      return `O Telinha parou com erro (${event.status}: ${event.message}).`;
  }
}

/* transmissoes em grupo */

function handleStreamEvent({ source, event }) {
  if (event.event === 'stats') {
    const fps = frameRate(source, event.role === 'sender' ? event.frames : event.presented);
    if (source === 'outgoing') {
      const { outgoing } = state.streams;
      Debug.update('outgoing', {
        label: 'Sua transmissão',
        rtt: event.rtt_ms,
        loss: event.loss,
        rows: senderRows(event, fps),
        summary: `Transmitindo ${outgoing ? outgoing.targetName : 'sua tela'} direto para quem está assistindo.`,
      });
    } else {
      const entry = state.streams.incoming.find((item) => item.sharerId === source);
      const name = entry ? entry.name : 'alguém';
      Debug.update(`watch-${source}`, {
        label: `Assistindo ${name}`,
        rtt: event.rtt_ms,
        loss: event.loss,
        rows: receiverRows(event, fps),
        summary: `Recebendo a tela de ${name} direto do computador dessa pessoa.`,
      });
    }
    return;
  }

  if (event.event === 'error') {
    Debug.appendLog(`[${source === 'outgoing' ? 'transmissão' : 'assistindo'}] erro ${event.stage}: ${event.status} ${event.message}`);
  }
  if (source !== 'outgoing') {
    return;
  }

  const info = state.outgoing;
  switch (event.event) {
    case 'step':
      state.roomFeedback = `${capitalize(event.text)}…`;
      break;
    case 'ready':
      state.roomFeedback = 'Transmissão no ar. Quem está na chamada já pode assistir.';
      if (state.pendingQuality && state.pendingQuality.preset !== 'auto') {
        api.streamSetQuality(state.pendingQuality);
      }
      state.pendingQuality = null;
      break;
    case 'target':
      if (info && info.pendingTarget) {
        info.target = info.pendingTarget;
        info.pendingTarget = null;
      }
      state.roomFeedback = `Agora mostrando ${info && info.target ? info.target.name : 'a nova tela'} em ${event.width}×${event.height}.`;
      if (info) {
        Share.syncStream(streamAudioState(info.target, info.audio));
      }
      break;
    case 'quality':
      state.roomFeedback = describeQuality(event);
      Share.streamFeedback(state.roomFeedback);
      break;
    case 'audio':
      if (info) {
        info.audio = { scope: event.scope, pid: event.pid, device: event.device ?? '' };
        Share.syncStream(streamAudioState(info.target, info.audio));
      }
      state.roomFeedback = `Som da transmissão: ${AUDIO_NAMES[event.scope] ?? event.scope}.`;
      Share.streamFeedback(state.roomFeedback);
      break;
    case 'command_failed':
      state.roomFeedback = describeCommandFailure(event);
      Share.streamFeedback(state.roomFeedback);
      if (info && event.command === 'switch_target') {
        info.pendingTarget = null;
      }
      if (info && event.command === 'set_audio') {
        Share.syncStream(streamAudioState(info.target, info.audio));
      }
      break;
    case 'error':
      state.roomFeedback = event.stage === 'audio' ? `Som: ${event.message}.` : describeError(event);
      break;
    default:
      return;
  }
  render();
}

function handleOutgoingEnded(info) {
  Debug.remove('outgoing');
  state.streamStats.delete('outgoing');
  state.pendingQuality = null;
  state.outgoing = null;
  Share.closeStream();
  if (Share.pickerMode() === 'switch') {
    Share.closePicker();
  }
  if (info.error) {
    state.roomFeedback = describeError(info.error);
    toast(state.roomFeedback);
  } else if (!info.stopped && info.code !== 0) {
    state.roomFeedback = `A transmissão parou sozinha (código ${info.code}).`;
  } else {
    state.roomFeedback = '';
  }
  render();
}

function handleIncomingEnded(info) {
  Debug.remove(`watch-${info.sharerId}`);
  state.streamStats.delete(info.sharerId);
  if (info.reason === 'failed') {
    toast(info.error && info.error.stage === 'timeout'
      ? `${info.name} não respondeu a tempo. Tente assistir de novo.`
      : `Não consegui assistir ${info.name}. Veja a depuração para os detalhes.`);
  } else if (info.reason === 'ended') {
    toast(`${info.name} parou de transmitir.`);
  }
  render();
}

/* conexao com codigo */

function activeManual() {
  const { manual } = state;
  return manual && MANUAL_ACTIVE.has(manual.phase) ? manual : null;
}

function newManual(fields) {
  return {
    phase: 'starting',
    role: 'share',
    running: false,
    stopping: false,
    stepText: '',
    feedback: '',
    message: '',
    outgoingCode: null,
    quality: { preset: 'auto', ...Share.QUALITY_PRESETS.auto },
    audio: { scope: 'system', pid: 0 },
    target: null,
    pendingTarget: null,
    fullscreen: false,
    saved: null,
    ...fields,
  };
}

function manualFailure(message, role) {
  state.manual = newManual({ role, phase: 'failed', message });
  openManualView();
}

function startManualShare() {
  Share.openPicker({
    mode: 'manual',
    title: 'O que você quer compartilhar?',
    confirmLabel: 'Gerar código',
    settings: true,
    quality: state.lastShare ? state.lastShare.quality : undefined,
    audio: state.lastShare ? state.lastShare.audio : undefined,
    onConfirm: async (choice) => {
      state.lastShare = choice;
      state.manualChoice = choice;
      try {
        await api.startShare(shareRequest(choice));
      } catch (error) {
        manualFailure(cleanError(error), 'share');
      }
    },
  });
}

async function startManualWatch() {
  state.manualChoice = null;
  try {
    await api.startWatch({});
  } catch (error) {
    manualFailure(cleanError(error), 'watch');
  }
}

function handleSessionStarted(info) {
  const manual = newManual({ role: info.role, running: true });
  const choice = state.manualChoice;
  if (info.role === 'share' && choice) {
    manual.quality = choice.quality;
    manual.audio = audioFromChoice(choice);
    manual.audioInputs = choice.audioInputs ?? [];
    manual.target = choice.selection;
  }
  state.manual = manual;
  state.streamStats.delete('manual');
  hideCodePanels();
  el.sessionStatus.textContent = 'Abrindo o Telinha…';
  renderSteps(0);
  openManualView();
}

async function stopManual() {
  const manual = activeManual();
  if (!manual || !manual.running || manual.stopping) {
    return;
  }
  manual.stopping = true;
  manual.feedback = 'Encerrando…';
  render();
  await api.stop();
}

function dismissManual() {
  if (state.manual && !MANUAL_ACTIVE.has(state.manual.phase)) {
    state.manual = null;
  }
  openHome();
}

function markManualFailed(message) {
  const { manual } = state;
  if (!manual || manual.phase === 'failed') {
    return;
  }
  manual.phase = 'failed';
  manual.message = message;
  manual.feedback = '';
  hideCodePanels();
  Share.closeStream();
}

function markManualEnded(message) {
  const { manual } = state;
  if (manual) {
    manual.phase = 'ended';
    manual.message = message;
    manual.feedback = '';
  }
}

function hideCodePanels() {
  el.panelCodeOut.hidden = true;
  el.panelCodeIn.hidden = true;
  el.codeIn.value = '';
  el.codeIn.disabled = false;
  el.submitCode.disabled = false;
  el.codeInError.textContent = '';
  el.copyFeedback.textContent = '';
}

function renderSteps(activeIndex) {
  const labels = state.manual && state.manual.role === 'share' ? SHARE_STEPS : WATCH_STEPS;
  el.steps.replaceChildren(...labels.map((label, index) => {
    const item = make('li', null, label);
    if (index < activeIndex) {
      item.className = 'done';
    } else if (index === activeIndex) {
      item.className = 'active';
    }
    return item;
  }));
}

async function copyOutgoingCode() {
  const code = state.manual ? state.manual.outgoingCode : null;
  if (!code) {
    return;
  }
  try {
    await api.copyText(code);
    el.copyFeedback.textContent = 'Copiado! Agora é só colar na conversa.';
  } catch (error) {
    el.copyFeedback.textContent = `Não consegui copiar: ${cleanError(error)}`;
  }
}

async function showOutgoingCode(event) {
  state.manual.outgoingCode = event.code;
  el.codeOut.textContent = event.code;
  if (event.kind === 'invite') {
    el.codeOutTitle.textContent = 'Mande este convite para quem vai assistir';
    el.codeOutHint.textContent = 'Ele já está copiado. Cole na conversa com a outra pessoa.';
    el.sessionStatus.textContent = 'Convite pronto.';
    renderSteps(1);
  } else {
    el.codeOutTitle.textContent = 'Mande esta resposta de volta';
    el.codeOutHint.textContent = 'Ela já está copiada. Quando a outra pessoa colar, a conexão abre sozinha.';
    el.sessionStatus.textContent = 'Esperando a outra pessoa colar a resposta…';
    el.panelCodeIn.hidden = true;
    renderSteps(2);
  }
  el.panelCodeOut.hidden = false;
  await copyOutgoingCode();
}

function showIncomingCode(event) {
  el.panelCodeIn.hidden = false;
  el.codeIn.disabled = false;
  el.submitCode.disabled = false;
  if (event.kind === 'invite') {
    el.codeInTitle.textContent = 'Cole o convite que você recebeu';
    el.codeInHint.textContent = 'Peça o convite para quem vai compartilhar a tela e cole aqui.';
    el.sessionStatus.textContent = 'Pronto para receber o convite.';
    renderSteps(1);
  } else {
    el.codeInTitle.textContent = 'Cole a resposta de quem vai assistir';
    el.codeInHint.textContent = 'Quando a outra pessoa mandar a resposta, cole aqui para conectar.';
    el.sessionStatus.textContent = 'Esperando a resposta da outra pessoa…';
    renderSteps(2);
  }
  el.codeIn.focus();
}

async function pasteFromClipboard() {
  const text = await api.readClipboard();
  el.codeIn.value = text.trim();
  el.codeInError.textContent = '';
  el.codeIn.focus();
}

async function submitCode() {
  if (el.submitCode.disabled || !state.manual) {
    return;
  }
  el.codeInError.textContent = '';
  const accepted = await api.submitCode(el.codeIn.value);
  if (!accepted) {
    el.codeInError.textContent = 'Isso não parece um código do Telinha. Copie o texto inteiro, que começa com TELINHA1.';
    return;
  }
  el.codeIn.disabled = true;
  el.submitCode.disabled = true;
  if (state.manual.role === 'share') {
    el.sessionStatus.textContent = 'Conectando…';
    renderSteps(3);
  } else {
    el.sessionStatus.textContent = 'Gerando a resposta…';
  }
}

function describeRejection(event) {
  const message = event.message || '';
  if (message.includes('mesma ponta')) {
    return state.manual && state.manual.role === 'share'
      ? 'Esse é o seu próprio convite. Cole a resposta que a outra pessoa mandou.'
      : 'Esse código é uma resposta, não um convite. Peça o convite para quem vai compartilhar.';
  }
  if (message.includes('truncated') || message.includes('corrupted')) {
    return 'O código chegou cortado ou alterado. Peça para a outra pessoa copiar de novo.';
  }
  if (message.includes('another version')) {
    return 'Esse código é de outra versão do Telinha. Os dois lados precisam da mesma versão.';
  }
  return 'Esse código não funcionou. Confira se copiou o texto inteiro.';
}

function applyManualState(value) {
  const { manual } = state;
  if (value === 'Connecting') {
    manual.stepText = 'Conectando os dois computadores…';
    el.sessionStatus.textContent = manual.stepText;
    renderSteps(3);
  } else if (value === 'Connected') {
    const first = manual.phase !== 'live';
    manual.phase = 'live';
    manual.feedback = '';
    hideCodePanels();
    if (first && manual.role === 'share' && manual.quality && manual.quality.preset !== 'auto') {
      api.setQuality(manual.quality);
    }
  } else if (value === 'Disconnected' && manual.phase === 'live') {
    manual.feedback = 'A conexão oscilou, tentando manter…';
  }
}

function handleManualEvent(event) {
  const { manual } = state;
  if (!manual) {
    return;
  }
  switch (event.event) {
    case 'step':
      manual.stepText = `${capitalize(event.text)}…`;
      if (manual.phase === 'starting') {
        el.sessionStatus.textContent = manual.stepText;
      }
      break;
    case 'code':
      showOutgoingCode(event);
      break;
    case 'need_code':
      showIncomingCode(event);
      break;
    case 'code_rejected':
      el.codeIn.disabled = false;
      el.submitCode.disabled = false;
      el.codeInError.textContent = describeRejection(event);
      break;
    case 'state':
      applyManualState(event.state);
      break;
    case 'stats': {
      const fps = frameRate('manual', event.role === 'sender' ? event.frames : event.presented);
      Debug.update('manual', {
        label: 'Conexão com código',
        rtt: event.rtt_ms,
        loss: event.loss,
        rows: event.role === 'sender' ? senderRows(event, fps) : receiverRows(event, fps),
        summary: manual.role === 'share'
          ? 'Você está transmitindo pela conexão com código.'
          : 'Você está assistindo pela conexão com código.',
      });
      renderCallPanel();
      return;
    }
    case 'target':
      if (manual.pendingTarget) {
        manual.target = manual.pendingTarget;
        manual.pendingTarget = null;
      }
      manual.feedback = `Agora mostrando ${manual.target ? manual.target.name : 'a nova tela'} em ${event.width}×${event.height}.`;
      Share.syncStream(streamAudioState(manual.target, manual.audio));
      break;
    case 'quality':
      manual.feedback = describeQuality(event);
      Share.streamFeedback(manual.feedback);
      break;
    case 'audio':
      manual.audio = { scope: event.scope, pid: event.pid, device: event.device ?? '' };
      manual.feedback = `Som da transmissão: ${AUDIO_NAMES[event.scope] ?? event.scope}.`;
      Share.streamFeedback(manual.feedback);
      Share.syncStream(streamAudioState(manual.target, manual.audio));
      break;
    case 'fullscreen':
      manual.fullscreen = Boolean(event.enabled);
      break;
    case 'command_failed':
      manual.feedback = describeCommandFailure(event);
      Share.streamFeedback(manual.feedback);
      if (event.command === 'switch_target') {
        manual.pendingTarget = null;
      } else if (event.command === 'set_audio') {
        Share.syncStream(streamAudioState(manual.target, manual.audio));
      }
      break;
    case 'error':
      Debug.appendLog(`erro ${event.stage}: ${event.status} ${event.message}`);
      if (event.stage === 'audio' && manual.phase === 'live') {
        manual.feedback = `Som: ${event.message}.`;
        Share.streamFeedback(manual.feedback);
      } else if (!manual.stopping) {
        markManualFailed(describeError(event));
      }
      break;
    default:
      return;
  }
  render();
}

function handleManualExit(info) {
  const { manual } = state;
  if (!manual) {
    return;
  }
  manual.running = false;
  if (manual.stopping) {
    markManualEnded('Você encerrou a sessão.');
  } else if (manual.phase !== 'failed') {
    if (info.code === 0) {
      markManualEnded(manual.role === 'watch' ? 'A janela do vídeo foi fechada.' : 'A transmissão terminou.');
    } else {
      markManualFailed(`O Telinha fechou inesperadamente (${info.error ?? `código ${info.code}`}).`);
    }
  }
  manual.stopping = false;
  manual.fullscreen = false;
  Share.closeStream();
  if (Share.pickerMode() === 'manual-switch') {
    Share.closePicker();
  }
  hideCodePanels();
  Debug.remove('manual');
  state.streamStats.delete('manual');
  render();
}

async function toggleManualFullscreen() {
  const manual = activeManual();
  if (!manual || manual.role !== 'watch' || manual.phase !== 'live') {
    return;
  }
  if (!(await api.setFullscreen(!manual.fullscreen))) {
    manual.feedback = 'Não consegui mudar a tela cheia agora.';
    render();
  }
}

function manualSwitch() {
  const manual = activeManual();
  if (!manual || manual.role !== 'share' || manual.phase !== 'live') {
    return;
  }
  Share.openPicker({
    mode: 'manual-switch',
    title: 'Trocar o que você está mostrando',
    confirmLabel: 'Trocar',
    settings: false,
    onConfirm: async ({ selection, audioInputs }) => {
      if (state.manual !== manual || manual.phase !== 'live') {
        return;
      }
      manual.audioInputs = audioInputs ?? manual.audioInputs;
      manual.pendingTarget = selection;
      manual.feedback = 'Trocando de tela…';
      render();
      if (!(await api.switchTarget({ kind: selection.kind, handle: selection.handle }))) {
        manual.pendingTarget = null;
        manual.feedback = 'Não consegui pedir a troca agora.';
        render();
      }
    },
  });
}

function manualSettings() {
  const manual = activeManual();
  if (!manual || manual.role !== 'share' || manual.phase !== 'live') {
    return;
  }
  Share.openStream({
    ...streamAudioState(manual.target, manual.audio),
    audioInputs: manual.audioInputs ?? [],
    quality: manual.quality,
    onQuality: async (quality) => {
      manual.quality = quality;
      Share.streamFeedback('Aplicando…');
      if (!(await api.setQuality(quality))) {
        Share.streamFeedback('Não consegui mudar a qualidade agora.');
      }
    },
    onAudio: async (scope, pid, device) => {
      Share.streamFeedback('Trocando o som…');
      if (!(await api.setAudio({ scope, pid, device }))) {
        Share.streamFeedback('Não consegui trocar o som agora.');
      }
    },
  });
}

function manualStageText(manual) {
  const share = manual.role === 'share';
  if (manual.phase === 'live') {
    return share
      ? ['Você está transmitindo', `Mostrando ${manual.target ? manual.target.name : 'sua tela'} · ${AUDIO_NAMES[manual.audio.scope] ?? 'som do computador'}`]
      : ['Assistindo', 'O vídeo abre numa janela do Telinha. F11, Alt+Enter ou dois cliques no vídeo alternam a tela cheia, Esc sai.'];
  }
  if (manual.phase === 'failed') {
    return ['Não deu certo', manual.message];
  }
  return ['Sessão encerrada', manual.message];
}

function renderManual() {
  const { manual } = state;
  const starting = Boolean(manual && manual.phase === 'starting');
  el.sessionTitle.textContent = manual && manual.role === 'watch'
    ? 'Assistindo a tela de outra pessoa'
    : 'Compartilhando sua tela';
  el.steps.hidden = !starting;
  el.sessionStatus.hidden = !starting;
  if (!starting) {
    el.panelCodeOut.hidden = true;
    el.panelCodeIn.hidden = true;
  }

  const showStage = Boolean(manual && !starting);
  el.stage.hidden = !showStage;
  if (!showStage) {
    return;
  }

  const live = manual.phase === 'live';
  const share = manual.role === 'share';
  el.stage.className = `stage${live ? ' live' : ''}${manual.phase === 'failed' ? ' failed' : ''}`;
  const [title, text] = manualStageText(manual);
  el.stageTitle.textContent = title;
  el.stageText.textContent = text;
  el.stageFeedback.textContent = live ? manual.feedback : '';

  const { saved } = manual;
  el.stageSaved.hidden = !saved;
  if (saved) {
    const current = findContact(saved.contact.id);
    const name = current ? current.name : saved.contact.name;
    el.stageSavedText.textContent = saved.created
      ? `Salvo nos contatos como “${name}”. Da próxima vez é só clicar no nome, sem código.`
      : `Essa pessoa já está nos contatos como “${name}”.`;
  }

  el.stageSwitch.hidden = !(live && share);
  el.stageSettings.hidden = !(live && share);
  el.stageFullscreen.hidden = !(live && !share);
  el.stageFullscreen.textContent = manual.fullscreen ? 'Sair da tela cheia' : 'Tela cheia';
  el.stageDebug.hidden = !live;
  el.stageStop.hidden = !live;
  el.stageStop.disabled = manual.stopping;
  el.stageDismiss.hidden = live;
}

function handleContactSaved({ contact, created }) {
  if (state.manual) {
    state.manual.saved = { contact, created };
    render();
  }
}

/* dialogos: nome, servidores, canais */

function openPrompt({ title, value, confirmLabel = 'Salvar', dangerLabel = null, onConfirm, onDanger = null }) {
  state.prompt = { onConfirm, onDanger };
  el.promptTitle.textContent = title;
  el.promptInput.value = value ?? '';
  el.promptSave.textContent = confirmLabel;
  el.promptDanger.hidden = !dangerLabel;
  if (dangerLabel) {
    el.promptDanger.textContent = dangerLabel;
  }
  el.promptDialog.hidden = false;
  el.promptInput.focus();
  el.promptInput.select();
}

function closePrompt() {
  el.promptDialog.hidden = true;
  state.prompt = null;
}

async function confirmPrompt() {
  const { prompt } = state;
  const value = el.promptInput.value;
  closePrompt();
  if (prompt) {
    await prompt.onConfirm(value);
  }
}

async function dangerPrompt() {
  const { prompt } = state;
  closePrompt();
  if (prompt && prompt.onDanger) {
    await prompt.onDanger();
  }
}

function openContactMenu(contact) {
  openPrompt({
    title: 'Contato',
    value: contact.name,
    confirmLabel: 'Salvar',
    dangerLabel: 'Remover',
    onConfirm: (name) => api.renameContact(contact.id, name),
    onDanger: async () => {
      if (window.confirm(`Remover ${contact.name} dos contatos? A conversa guardada também some.`)) {
        await api.removeContact(contact.id);
        state.unread.delete(contact.id);
      }
    },
  });
}

function openServerDialog() {
  el.serverCreateName.value = `Servidor de ${state.profile.name}`.slice(0, 40);
  el.serverJoinCode.value = '';
  el.serverDialogError.textContent = '';
  el.serverDialog.hidden = false;
  el.serverCreateName.focus();
  el.serverCreateName.select();
}

async function createServer() {
  try {
    const server = await api.createServer(el.serverCreateName.value);
    el.serverDialog.hidden = true;
    state.servers = await api.listServers();
    openServer(server.id);
    openInvite(server.id);
  } catch (error) {
    el.serverDialogError.textContent = cleanError(error);
  }
}

async function joinServer(code) {
  const fromDialog = !el.serverDialog.hidden;
  try {
    const result = await api.joinServer(code ?? el.serverJoinCode.value);
    el.serverDialog.hidden = true;
    state.servers = await api.listServers();
    openServer(result.server.id);
    toast(result.created
      ? `Você entrou em ${result.server.name}. Os canais aparecem quando alguém de lá estiver online.`
      : `Você já está em ${result.server.name}.`);
  } catch (error) {
    if (fromDialog) {
      el.serverDialogError.textContent = cleanError(error);
    } else {
      toast(cleanError(error));
    }
  }
}

async function openInvite(serverId) {
  const server = findServer(serverId);
  const code = server ? await api.inviteCode(serverId) : null;
  if (!server || !code) {
    return;
  }
  state.invite = { serverId, code };
  el.inviteTitle.textContent = `Convidar para ${server.name}`;
  el.inviteCode.textContent = code;
  el.inviteFeedback.textContent = '';
  el.inviteContactsTitle.hidden = state.contacts.length === 0;
  el.inviteContacts.replaceChildren(...state.contacts.map((contact) => {
    const item = make('li');
    const send = actionButton('Enviar', 'primary small', async () => {
      send.disabled = true;
      try {
        const message = await api.sendChat({
          spaceId: contact.id,
          channelId: null,
          text: `Entre no meu servidor ${server.name} no Telinha: ${code}`,
        });
        if (message) {
          Chat.receive(contact.id, message);
        }
        send.textContent = message && message.delivered === false ? 'Sem conexão' : 'Enviado';
      } catch (error) {
        send.disabled = false;
        toast(cleanError(error));
      }
    });
    item.append(avatarNode(contact.name, contactStatus(contact) !== 'offline', '', state.contactMembers.get(contact.id)),
      make('span', 'nav-name', contact.name), send);
    return item;
  }));
  el.inviteDialog.hidden = false;
}

async function copyInvite() {
  if (!state.invite) {
    return;
  }
  await api.copyText(state.invite.code);
  el.inviteFeedback.textContent = 'Copiado! Mande para quem você quer no servidor.';
}

function openChannelDialog(serverId, channel = null, kind = 'text') {
  const chosenKind = channel ? channel.kind : kind;
  state.channelEdit = { serverId, channelId: channel ? channel.id : null };
  el.channelTitle.textContent = channel ? 'Editar canal' : 'Criar canal';
  el.channelKind.hidden = Boolean(channel);
  for (const input of el.channelKind.querySelectorAll('input[name="channel-kind"]')) {
    input.checked = input.value === chosenKind;
  }
  el.channelName.value = channel ? channel.name : '';
  el.channelName.placeholder = chosenKind === 'voice' ? 'Nova sala' : 'novo-canal';
  el.channelRemove.hidden = !channel;
  el.channelSave.textContent = channel ? 'Salvar' : 'Criar canal';
  el.channelDialog.hidden = false;
  el.channelName.focus();
}

function closeChannelDialog() {
  el.channelDialog.hidden = true;
  state.channelEdit = null;
}

async function saveChannel() {
  const edit = state.channelEdit;
  if (!edit) {
    return;
  }
  const name = el.channelName.value;
  closeChannelDialog();
  if (edit.channelId) {
    await api.renameChannel(edit.serverId, edit.channelId, name);
    return;
  }
  const checked = el.channelKind.querySelector('input[name="channel-kind"]:checked');
  const channel = await api.addChannel(edit.serverId, checked ? checked.value : 'text', name);
  if (channel) {
    state.servers = await api.listServers();
    openChannel(edit.serverId, channel.id);
  }
}

async function removeChannel() {
  const edit = state.channelEdit;
  if (!edit || !edit.channelId) {
    return;
  }
  const channel = findChannel(findServer(edit.serverId), edit.channelId);
  closeChannelDialog();
  if (channel && window.confirm(`Excluir ${channel.name} para todo mundo do servidor? As mensagens guardadas neste computador somem.`)) {
    await api.removeChannel(edit.serverId, edit.channelId);
  }
}

async function serverAction(action) {
  el.serverMenu.hidden = true;
  const server = currentServer();
  if (!server) {
    return;
  }
  if (action === 'invite') {
    openInvite(server.id);
  } else if (action === 'text' || action === 'voice') {
    openChannelDialog(server.id, null, action);
  } else if (action === 'rename') {
    openPrompt({
      title: 'Renomear servidor',
      value: server.name,
      onConfirm: (name) => api.renameServer(server.id, name),
    });
  } else if (action === 'leave'
    && window.confirm(`Sair de ${server.name}? As conversas guardadas neste computador somem e você só volta com um convite novo.`)) {
    await api.leaveServer(server.id);
  }
}

/* configuracoes */

function fillSelect(select, devices, selectedId, defaultLabel) {
  const options = [new Option(defaultLabel, '')];
  devices
    .filter((device) => device.deviceId && device.deviceId !== 'default' && device.deviceId !== 'communications')
    .forEach((device, index) => options.push(new Option(device.label || `Dispositivo ${index + 1}`, device.deviceId)));
  select.replaceChildren(...options);
  select.value = options.some((option) => option.value === selectedId) ? selectedId : '';
}

async function fillDevices() {
  const devices = await Call.listDevices();
  const { settings } = Call.state();
  fillSelect(el.settingsMic, devices.mics, settings.micId, 'Microfone padrão do Windows');
  fillSelect(el.settingsSpeaker, devices.speakers, settings.speakerId, 'Saída padrão do Windows');
  fillSelect(el.settingsCamera, devices.cameras, settings.cameraId, 'Primeira câmera encontrada');
  el.settingsBlur.checked = settings.blur;
}

async function restartMicTest() {
  const result = await Call.startMicTest((level) => {
    el.settingsMeterFill.style.width = `${Math.round(level * 100)}%`;
  });
  el.settingsMicStatus.textContent = result.ok
    ? 'Fale alguma coisa: a barra verde mostra o volume que chega do microfone.'
    : result.message;
}

function stopSettingsPreview() {
  Call.stopPreview();
  el.settingsPreview.srcObject = null;
  el.settingsPreviewEmpty.hidden = false;
  el.settingsPreviewToggle.textContent = 'Testar câmera';
  state.previewing = false;
}

async function toggleSettingsPreview() {
  if (state.previewing) {
    stopSettingsPreview();
    return;
  }
  const result = await Call.startPreview(el.settingsPreview);
  if (!result.ok) {
    el.settingsCameraStatus.textContent = result.message;
    return;
  }
  state.previewing = true;
  el.settingsPreviewEmpty.hidden = true;
  el.settingsPreviewToggle.textContent = 'Parar teste';
  el.settingsCameraStatus.textContent = result.blur
    ? ''
    : 'Desfocar o fundo precisa de uma câmera com os efeitos do Windows (Windows Studio Effects).';
  fillDevices();
}

async function openSettings() {
  el.settingsName.value = state.profile.name;
  renderSettingsAvatar();
  el.settingsCameraStatus.textContent = '';
  stopSettingsPreview();
  el.settingsDialog.hidden = false;
  await fillDevices();
  await restartMicTest();
  await fillDevices();
}

function closeSettings() {
  Call.stopMicTest();
  stopSettingsPreview();
  el.settingsMeterFill.style.width = '0%';
  el.settingsDialog.hidden = true;
}

async function saveName() {
  state.profile = await api.setName(el.settingsName.value);
  el.settingsName.value = state.profile.name;
  toast('Nome salvo.');
  render();
}

/* menu de contexto, volumes, foco e tela cheia */

const STAGE_HEIGHT_KEY = 'telinha.stageHeight';
// Mesmo valor de --stage-limit no CSS: o que sobra para o cabecalho, os controles da sala,
// um pedaco das mensagens e a caixa de texto.
const STAGE_RESERVED_PX = 340;

function closeContextMenu() {
  el.contextMenu.hidden = true;
  el.contextMenu.replaceChildren();
}

function menuItem(item) {
  if (item.type === 'separator') {
    return make('div', 'menu-separator');
  }
  if (item.type === 'header') {
    return make('div', 'menu-header', item.label);
  }
  if (item.type === 'slider') {
    const wrapper = make('label', 'menu-slider');
    const value = make('span', 'menu-slider-value', `${item.value}%`);
    const head = make('span', 'menu-slider-head');
    head.append(make('span', null, item.label), value);
    const input = make('input');
    input.type = 'range';
    input.min = String(item.min);
    input.max = String(item.max);
    input.step = '1';
    input.value = String(item.value);
    input.addEventListener('input', () => {
      value.textContent = `${input.value}%`;
      item.onInput(Number(input.value));
    });
    wrapper.append(head, input);
    return wrapper;
  }
  if (item.type === 'check') {
    const button = make('button', 'menu-check');
    button.type = 'button';
    const box = make('span', item.checked ? 'menu-box checked' : 'menu-box');
    button.append(make('span', null, item.label), box);
    button.addEventListener('click', () => {
      item.checked = !item.checked;
      box.classList.toggle('checked', item.checked);
      item.onToggle(item.checked);
    });
    return button;
  }
  const button = make('button', item.danger ? 'menu-action danger' : 'menu-action', item.label);
  button.type = 'button';
  button.addEventListener('click', () => {
    closeContextMenu();
    item.onSelect();
  });
  return button;
}

function openContextMenu(event, items) {
  el.contextMenu.replaceChildren(...items.map(menuItem));
  el.contextMenu.hidden = false;
  const { width, height } = el.contextMenu.getBoundingClientRect();
  const left = Math.max(8, Math.min(event.clientX, window.innerWidth - width - 8));
  const top = Math.max(8, Math.min(event.clientY, window.innerHeight - height - 8));
  el.contextMenu.style.left = `${left}px`;
  el.contextMenu.style.top = `${top}px`;
}

function personFromVoice(spaceId, roomId, person) {
  const inMyRoom = Boolean(state.voice && state.voice.spaceId === spaceId && state.voice.roomId === roomId);
  return {
    id: person.id,
    memberId: person.memberId ?? null,
    name: person.name,
    self: Boolean(person.self),
    contactId: findContact(spaceId) ? spaceId : null,
    inRoom: inMyRoom && !person.self,
  };
}

function personFromTile(info) {
  const { voice } = state;
  if (!voice) {
    return null;
  }
  const peer = peersOf(voice.spaceId).find((item) => item.id === info.id) ?? null;
  return {
    id: info.id,
    memberId: info.self ? state.profile.memberId : (peer ? peer.memberId : null),
    name: info.name,
    self: Boolean(info.self),
    contactId: findContact(voice.spaceId) ? voice.spaceId : null,
    inRoom: !info.self,
  };
}

function personFromContact(contact) {
  const peers = peersOf(contact.id);
  const peer = peers.find((item) => item.voice && item.voice.roomId === DM_ROOM) ?? peers[0] ?? null;
  const inMyRoom = Boolean(state.voice && state.voice.spaceId === contact.id && peer && peer.voice
    && peer.voice.roomId === DM_ROOM);
  return {
    id: peer ? peer.id : null,
    memberId: peer ? peer.memberId : null,
    name: contact.name,
    self: false,
    contactId: contact.id,
    inRoom: inMyRoom,
  };
}

function personFromMember(server, member) {
  const inMyRoom = Boolean(!member.self && member.voice && state.voice && state.voice.spaceId === server.id
    && member.voice.roomId === state.voice.roomId);
  return {
    id: member.peerId ?? null,
    memberId: member.memberId ?? null,
    name: member.name,
    self: Boolean(member.self),
    contactId: null,
    inRoom: inMyRoom,
  };
}

function openPersonMenu(event, person) {
  if (!person) {
    return;
  }
  const items = [{ type: 'header', label: person.self ? `${person.name} (você)` : person.name }];
  const contact = person.contactId ? findContact(person.contactId) : null;
  if (contact) {
    items.push({ type: 'action', label: 'Mensagem', onSelect: () => openContact(contact.id) });
    if (!(state.voice && state.voice.spaceId === contact.id)) {
      items.push({ type: 'action', label: 'Iniciar chamada', onSelect: () => startCall(contact.id) });
    }
  }

  if (person.inRoom && person.id) {
    const key = Call.prefsKey({ id: person.id, memberId: person.memberId });
    const prefs = Call.personPrefs(key);
    items.push({ type: 'separator' });
    items.push({
      type: 'slider',
      label: 'Volume do usuário',
      value: prefs.volume,
      min: 0,
      max: 200,
      onInput: (value) => Call.setPersonPrefs(key, { volume: value }),
    });
    if (state.streams.incoming.some((entry) => entry.sharerId === person.id)) {
      items.push({
        type: 'slider',
        label: 'Volume da transmissão',
        value: prefs.streamVolume,
        min: 0,
        max: 200,
        onInput: (value) => {
          Call.setPersonPrefs(key, { streamVolume: value });
          StreamView.refreshAudio();
          api.streamVolume(person.id, value);
        },
      });
    }
    items.push({
      type: 'check',
      label: 'Silenciar',
      checked: prefs.muted,
      onToggle: (checked) => Call.setPersonPrefs(key, { muted: checked }),
    });
    items.push({
      type: 'check',
      label: 'Desativar vídeo',
      checked: prefs.videoHidden,
      onToggle: (checked) => Call.setPersonPrefs(key, { videoHidden: checked }),
    });
  }

  if (contact) {
    items.push({ type: 'separator' });
    items.push({ type: 'action', label: 'Renomear ou remover contato', onSelect: () => openContactMenu(contact) });
  }
  if (person.memberId) {
    items.push({ type: 'separator' });
    items.push({ type: 'action', label: 'Copiar ID do usuário', onSelect: () => api.copyText(person.memberId) });
  }
  if (items.length > 1) {
    openContextMenu(event, items);
  }
}

function applyStreamVolumes() {
  const live = new Set();
  for (const entry of state.streams.incoming) {
    if (entry.state !== 'live') {
      continue;
    }
    live.add(entry.sharerId);
    if (state.volumeApplied.has(entry.sharerId)) {
      continue;
    }
    state.volumeApplied.add(entry.sharerId);
    const peer = peersOf(entry.spaceId).find((item) => item.id === entry.sharerId);
    const prefs = Call.personPrefs(Call.prefsKey({ id: entry.sharerId, memberId: peer ? peer.memberId : null }));
    if (prefs.streamVolume !== 100) {
      api.streamVolume(entry.sharerId, prefs.streamVolume);
    }
  }
  for (const id of [...state.volumeApplied]) {
    if (!live.has(id)) {
      state.volumeApplied.delete(id);
    }
  }
}

function toggleFocus(id) {
  state.focusTile = state.focusTile === id ? null : id;
  if (state.voice) {
    renderStage();
  }
}

async function openTileViewer(info) {
  const stream = info.stream || Call.videoStream(info.id);
  if (!stream) {
    return;
  }
  el.tileViewerVideo.srcObject = stream;
  el.tileViewerVideo.classList.toggle('mirror', Boolean(info.self));
  el.tileViewerName.textContent = info.self ? `${info.name} (você)` : info.name;
  el.tileViewer.hidden = false;
  el.tileViewerVideo.play().catch(() => {});
  try {
    await el.tileViewer.requestFullscreen();
  } catch {
    // sem tela cheia do sistema, a camera fica ocupando a janela inteira
  }
}

function closeTileViewer() {
  if (document.fullscreenElement) {
    document.exitFullscreen().catch(() => {});
  }
  el.tileViewer.hidden = true;
  el.tileViewerVideo.srcObject = null;
}

function applyStageHeight(height) {
  if (!height) {
    el.roomTiles.style.height = '';
    el.roomTiles.style.removeProperty('--stage-cap');
    return;
  }
  el.roomTiles.style.setProperty('--stage-cap', `min(${height}px, var(--stage-limit))`);
  el.roomTiles.style.height = 'var(--stage-cap)';
}

function bindStageResize() {
  try {
    state.stageHeight = Number(localStorage.getItem(STAGE_HEIGHT_KEY)) || null;
  } catch {
    state.stageHeight = null;
  }
  applyStageHeight(state.stageHeight);

  let startY = 0;
  let startHeight = 0;
  const move = (event) => {
    const limit = Math.max(140, window.innerHeight - STAGE_RESERVED_PX);
    state.stageHeight = Math.round(Math.max(140, Math.min(limit, startHeight + event.clientY - startY)));
    applyStageHeight(state.stageHeight);
  };
  const stop = () => {
    document.removeEventListener('mousemove', move);
    document.removeEventListener('mouseup', stop);
    document.body.classList.remove('resizing');
    try {
      localStorage.setItem(STAGE_HEIGHT_KEY, String(state.stageHeight ?? ''));
    } catch {
      // sem armazenamento local, o tamanho vale so nesta execucao
    }
  };
  el.roomResizer.addEventListener('mousedown', (event) => {
    event.preventDefault();
    startY = event.clientY;
    startHeight = el.roomTiles.getBoundingClientRect().height;
    document.body.classList.add('resizing');
    document.addEventListener('mousemove', move);
    document.addEventListener('mouseup', stop);
  });
  el.roomResizer.addEventListener('dblclick', () => {
    state.stageHeight = null;
    applyStageHeight(null);
    try {
      localStorage.removeItem(STAGE_HEIGHT_KEY);
    } catch {
      // nada guardado
    }
  });
}

/* avatar */

function renderSettingsAvatar() {
  const node = el.settingsAvatar;
  node.replaceChildren();
  node.classList.toggle('has-image', Boolean(state.profile.avatar));
  node.style.background = Call.avatarColor(state.profile.name);
  if (state.profile.avatar) {
    const image = make('img');
    image.src = state.profile.avatar;
    image.alt = '';
    node.append(image);
  } else {
    node.textContent = initial(state.profile.name);
  }
  el.settingsAvatarRemove.hidden = !state.profile.avatar;
}

async function removeAvatar() {
  state.profile = await api.removeAvatar();
  renderSettingsAvatar();
  refreshConversation();
  render();
}

async function openAvatarPicker() {
  const recents = await api.recentAvatars().catch(() => []);
  AvatarEditor.openPicker({
    recents,
    hasAvatar: Boolean(state.profile.avatar),
    onApply: async (mime, data) => {
      try {
        state.profile = await api.setAvatar({ mime, data });
        toast('Avatar atualizado.');
      } catch (error) {
        toast(cleanError(error));
      }
      renderSettingsAvatar();
      refreshConversation();
      render();
    },
    onRecent: async (hash) => {
      state.profile = await api.useRecentAvatar(hash);
      renderSettingsAvatar();
      refreshConversation();
      render();
    },
    onRemove: removeAvatar,
  });
}

/* transmissoes dentro do app */

function streamVolumeFor(sharerId) {
  const entry = state.streams.incoming.find((item) => item.sharerId === sharerId);
  const peer = entry ? peersOf(entry.spaceId).find((item) => item.id === sharerId) : null;
  return Call.personPrefs(Call.prefsKey({ id: sharerId, memberId: peer ? peer.memberId : null })).streamVolume;
}

const THUMB_INTERVAL_MS = 4000;
const THUMB_MAX_AGE_MS = 60000;
const THUMB_WIDTH = 480;
let thumbTimer = null;
let thumbCanvas = null;

// Quem transmite manda um retrato pequeno de tempos em tempos, para quem ainda nao clicou em assistir.
function captureThumb() {
  const video = StreamView.video(StreamView.SELF);
  if (!video || !video.videoWidth || !video.videoHeight) {
    return null;
  }
  if (!thumbCanvas) {
    thumbCanvas = document.createElement('canvas');
  }
  const scale = Math.min(1, THUMB_WIDTH / video.videoWidth);
  thumbCanvas.width = Math.max(2, Math.round(video.videoWidth * scale));
  thumbCanvas.height = Math.max(2, Math.round(video.videoHeight * scale));
  thumbCanvas.getContext('2d').drawImage(video, 0, 0, thumbCanvas.width, thumbCanvas.height);
  return thumbCanvas.toDataURL('image/jpeg', 0.55);
}

function sendThumb() {
  if (Call.participants().length === 0) {
    return;
  }
  const image = captureThumb();
  if (image) {
    api.publishStreamThumb(image).catch(() => {});
  }
}

function startThumbLoop() {
  stopThumbLoop();
  thumbTimer = setInterval(sendThumb, THUMB_INTERVAL_MS);
  setTimeout(sendThumb, 1500);
}

function stopThumbLoop() {
  clearInterval(thumbTimer);
  thumbTimer = null;
}

function handleStreamThumb({ sharerId, image }) {
  state.streamThumbs.set(sharerId, { image, at: Date.now() });
  render();
}

function streamThumbFor(sharerId) {
  if (state.streams.incoming.some((entry) => entry.sharerId === sharerId)) {
    return null;
  }
  const entry = state.streamThumbs.get(sharerId);
  if (!entry) {
    return null;
  }
  if (Date.now() - entry.at > THUMB_MAX_AGE_MS) {
    state.streamThumbs.delete(sharerId);
    return null;
  }
  return entry.image;
}

function streamTilesInfo() {
  const { voice } = state;
  if (!voice) {
    return [];
  }
  return state.streams.incoming
    .filter((entry) => entry.spaceId === voice.spaceId && entry.roomId === voice.roomId)
    .map((entry) => ({
      id: `stream:${entry.sharerId}`,
      sharerId: entry.sharerId,
      name: entry.name,
      mode: entry.mode,
      state: entry.mode === 'embedded' ? (StreamView.state(entry.sharerId) || 'connecting') : entry.state,
      video: entry.mode === 'embedded' ? StreamView.video(entry.sharerId) : null,
    }));
}

function openStreamViewer(stream) {
  const media = StreamView.mediaStream(stream.sharerId);
  if (media) {
    openTileViewer({ id: stream.id, name: `Tela de ${stream.name}`, self: false, stream: media });
  }
}

function handleStreamViewStats(sharerId, stats) {
  if (sharerId === StreamView.SELF) {
    return;
  }
  const entry = state.streams.incoming.find((item) => item.sharerId === sharerId);
  const name = entry ? entry.name : 'alguém';
  Debug.update(`watch-${sharerId}`, {
    label: `Assistindo ${name}`,
    rtt: stats.rtt,
    loss: stats.loss,
    rows: [
      ['Quadros por segundo', finite(stats.fps, 0)],
      ['Resolução', stats.width ? `${stats.width}×${stats.height}` : '—'],
      ['Ping', finite(stats.rtt, 0, ' ms')],
      ['Perda de pacotes', finite(stats.loss * 100, 2, ' %')],
      ['Taxa recebida', finite(Number.isFinite(stats.bitrate) ? stats.bitrate / 1e6 : NaN, 2, ' Mbps')],
      ['Atraso do buffer', finite(stats.jitter, 0, ' ms')],
      ['Quadros descartados', formatCount(stats.dropped)],
      ['Decodificador', stats.decoder || '—'],
    ],
    summary: `Recebendo a tela de ${name} direto do computador dessa pessoa, dentro do app.`,
  });
}

/* painel da conexao */

function formatElapsed(milliseconds) {
  const total = Math.max(0, Math.floor(milliseconds / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = String(total % 60).padStart(2, '0');
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, '0')}:${seconds}` : `${minutes}:${seconds}`;
}

function updateVoiceTimers() {
  if (!state.voice) {
    return;
  }
  const text = formatElapsed(Date.now() - state.voiceSince);
  for (const node of document.querySelectorAll('.voice-timer')) {
    node.textContent = text;
  }
}

function setPopoverTab(tab) {
  for (const button of el.voicePopover.querySelectorAll('[data-popover-tab]')) {
    button.setAttribute('aria-selected', String(button.dataset.popoverTab === tab));
  }
  el.popoverConnection.hidden = tab !== 'connection';
  el.popoverPrivacy.hidden = tab !== 'privacy';
}

function closeVoicePopover() {
  el.voicePopover.hidden = true;
}

function drawPopoverGraph(samples) {
  const canvas = el.popoverGraph;
  const ratio = window.devicePixelRatio || 1;
  const width = canvas.clientWidth || 210;
  const height = canvas.clientHeight || 64;
  canvas.width = Math.round(width * ratio);
  canvas.height = Math.round(height * ratio);
  const context = canvas.getContext('2d');
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.clearRect(0, 0, width, height);

  const styles = getComputedStyle(document.documentElement);
  const plotWidth = width - 26;
  const peak = Math.max(100, ...samples.map((sample) => sample.rtt ?? 0));
  const scale = Math.ceil(peak / 100) * 100;
  context.font = '10px Segoe UI, sans-serif';
  context.fillStyle = styles.getPropertyValue('--muted').trim();
  context.strokeStyle = styles.getPropertyValue('--line-soft').trim();
  context.lineWidth = 1;
  [scale, scale / 2, 0].forEach((value, index) => {
    const y = 5 + ((height - 10) * index) / 2;
    context.beginPath();
    context.moveTo(0, y);
    context.lineTo(plotWidth, y);
    context.stroke();
    context.fillText(String(Math.round(value)), plotWidth + 4, y + 3);
  });

  context.strokeStyle = styles.getPropertyValue('--accent').trim();
  context.lineWidth = 1.5;
  context.beginPath();
  let started = false;
  const steps = Math.max(1, samples.length - 1);
  samples.forEach((sample, index) => {
    if (sample.rtt === null) {
      started = false;
      return;
    }
    const x = (plotWidth * index) / steps;
    const y = 5 + (height - 10) * (1 - Math.min(sample.rtt, scale) / scale);
    if (started) {
      context.lineTo(x, y);
    } else {
      context.moveTo(x, y);
      started = true;
    }
  });
  context.stroke();

  const labels = [];
  if (samples.length > 1) {
    for (let index = 0; index < 4; index += 1) {
      const sample = samples[Math.round((steps * index) / 3)];
      labels.push(new Date(sample.at).toLocaleTimeString('pt-BR', { hour: '2-digit', minute: '2-digit' }));
    }
  }
  el.popoverTimes.replaceChildren(...labels.map((label) => make('span', null, label)));
}

function renderVoicePopover() {
  if (el.voicePopover.hidden || !state.voice) {
    return;
  }
  const samples = Debug.samples('call');
  const pings = samples.map((sample) => sample.rtt).filter((value) => value !== null);
  const last = samples.length > 0 ? samples[samples.length - 1] : null;
  el.popoverPlace.textContent = `${placeName(state.voice)} · direto (P2P)`;
  el.popoverAverage.textContent = pings.length > 0
    ? `${Math.round(pings.reduce((sum, value) => sum + value, 0) / pings.length)} ms`
    : '—';
  el.popoverLast.textContent = last && last.rtt !== null ? `${Math.round(last.rtt)} ms` : '—';
  el.popoverLoss.textContent = last && last.loss !== null ? `${last.loss.toFixed(1)}%` : '—';
  drawPopoverGraph(samples);
}

function toggleVoicePopover() {
  if (!state.voice) {
    callPanelDebug();
    return;
  }
  if (!el.voicePopover.hidden) {
    closeVoicePopover();
    return;
  }
  setPopoverTab('connection');
  el.voicePopover.hidden = false;
  renderVoicePopover();
  const panel = el.callPanel.getBoundingClientRect();
  const { height } = el.voicePopover.getBoundingClientRect();
  el.voicePopover.style.left = `${Math.round(panel.left + 8)}px`;
  el.voicePopover.style.top = `${Math.max(8, Math.round(panel.top - height - 8))}px`;
}

function bindVoicePopover() {
  for (const button of el.voicePopover.querySelectorAll('[data-popover-tab]')) {
    button.addEventListener('click', () => setPopoverTab(button.dataset.popoverTab));
  }
  el.popoverDebug.addEventListener('click', () => {
    closeVoicePopover();
    Debug.open('call');
  });
  el.popoverCopy.addEventListener('click', async () => {
    await api.copyText(Debug.logText());
    el.popoverCopy.textContent = 'Copiado';
    setTimeout(() => {
      el.popoverCopy.textContent = 'Copiar registro';
    }, 1500);
  });
}

/* mesa de sons */

function arrayBufferToBase64(buffer) {
  const bytes = new Uint8Array(buffer);
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return btoa(binary);
}

function base64ToArrayBuffer(base64) {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes.buffer;
}

async function loadSoundData(id) {
  const base64 = await api.soundData(id);
  return base64 ? base64ToArrayBuffer(base64) : null;
}

function closeSoundboard() {
  el.soundboard.hidden = true;
}

async function refreshSounds() {
  state.sounds = await api.listSounds();
  renderSoundboard();
}

async function playSoundboard(sound, button) {
  button.classList.add('playing');
  setTimeout(() => button.classList.remove('playing'), 600);
  try {
    await Call.playSound(sound, loadSoundData);
  } catch (error) {
    toast(`Não consegui tocar ${sound.name}: ${cleanError(error)}`);
  }
}

function openSoundMenu(event, sound) {
  openContextMenu(event, [
    { type: 'header', label: sound.name },
    { type: 'action', label: 'Tocar', onSelect: () => Call.playSound(sound, loadSoundData).catch(() => {}) },
    {
      type: 'action',
      label: 'Renomear',
      onSelect: () => openPrompt({
        title: 'Renomear som',
        value: sound.name,
        onConfirm: async (name) => {
          await api.renameSound(sound.id, name);
          await refreshSounds();
        },
      }),
    },
    {
      type: 'action',
      label: 'Remover',
      danger: true,
      onSelect: async () => {
        await api.removeSound(sound.id);
        Call.forgetSound(sound.id);
        await refreshSounds();
      },
    },
  ]);
}

function renderSoundboard() {
  const filter = el.soundboardSearch.value.trim().toLowerCase();
  const buttons = state.sounds
    .filter((sound) => !filter || sound.name.toLowerCase().includes(filter))
    .map((sound) => {
      const button = make('button', 'sound-button');
      button.type = 'button';
      button.title = `Tocar ${sound.name}`;
      const glyph = make('span', 'sound-glyph');
      glyph.innerHTML = ICONS.speaker;
      button.append(glyph, make('span', 'sound-name', sound.name));
      button.addEventListener('click', () => playSoundboard(sound, button));
      button.addEventListener('contextmenu', (event) => {
        event.preventDefault();
        openSoundMenu(event, sound);
      });
      return button;
    });
  const add = make('button', 'sound-button sound-add');
  add.type = 'button';
  add.title = 'Adicionar som';
  add.append(make('span', 'sound-glyph', '+'), make('span', 'sound-name', 'Adicionar som'));
  add.addEventListener('click', () => el.soundboardFile.click());
  el.soundboardGrid.replaceChildren(...buttons, add);
  el.soundboardHint.textContent = state.voice
    ? 'Clique para tocar para todo mundo na chamada. Botão direito renomeia ou remove.'
    : 'Fora de uma chamada o som toca só para você.';
}

function toggleSoundboard() {
  if (!el.soundboard.hidden) {
    closeSoundboard();
    return;
  }
  el.soundboardSearch.value = '';
  el.soundboardVolume.value = String(Math.round(Call.getSoundVolume() * 100));
  el.soundboard.hidden = false;
  renderSoundboard();
  const panel = el.callPanel.getBoundingClientRect();
  const { height } = el.soundboard.getBoundingClientRect();
  el.soundboard.style.left = `${Math.round(panel.left + 8)}px`;
  el.soundboard.style.top = `${Math.max(8, Math.round(panel.top - height - 8))}px`;
  el.soundboardSearch.focus();
}

async function addSoundFromFile(file) {
  el.soundboardFile.value = '';
  if (!file) {
    return;
  }
  if (!file.type.startsWith('audio/')) {
    toast('Escolha um arquivo de áudio, como MP3, OGG ou WAV.');
    return;
  }
  if (file.size > 1024 * 1024) {
    toast('O som precisa ter no máximo 1 MB.');
    return;
  }
  let buffer;
  try {
    buffer = await file.arrayBuffer();
    if (await Call.soundDuration(buffer) > 10.5) {
      toast('O som precisa ter no máximo 10 segundos.');
      return;
    }
  } catch {
    toast('Não consegui ler esse áudio.');
    return;
  }
  openPrompt({
    title: 'Nome do som',
    value: file.name.replace(/\.[^.]+$/, '').slice(0, 32),
    confirmLabel: 'Adicionar',
    onConfirm: async (name) => {
      try {
        await api.addSound({ name, mime: file.type, data: arrayBufferToBase64(buffer) });
        await refreshSounds();
      } catch (error) {
        toast(cleanError(error));
      }
    },
  });
}

function bindSoundboard() {
  el.soundboardSearch.addEventListener('input', renderSoundboard);
  el.soundboardVolume.addEventListener('input', () => Call.setSoundVolume(Number(el.soundboardVolume.value) / 100));
  el.soundboardFile.addEventListener('change', () => addSoundFromFile(el.soundboardFile.files[0]));
}

/* eventos */

function handlePresence({ spaceId, peers }) {
  state.presence.set(spaceId, peers);
  Call.setPresence(spaceId, peers);
  const call = state.incomingCall;
  if (call && call.contactId === spaceId
    && !peers.some((peer) => peer.voice && peer.voice.roomId === DM_ROOM)) {
    closeIncomingCall();
  }
  refreshConversation();
  render();
}

function handleContacts(contacts) {
  state.contacts = contacts;
  if (state.nav.space === 'home' && state.nav.view === 'chat' && !findContact(state.nav.id)) {
    openHome();
    return;
  }
  refreshConversation();
  render();
}

function handleServers(servers) {
  state.servers = servers;
  if (state.nav.space !== 'home') {
    const server = currentServer();
    if (!server) {
      openHome();
      return;
    }
    const channelMissing = state.nav.id && !findChannel(server, state.nav.id);
    if (channelMissing || (!state.nav.id && server.channels.length > 0)) {
      openServer(server.id);
      return;
    }
  }
  refreshConversation();
  render();
}

function handleVoice(voice) {
  const previous = state.voice;
  if (voice && (!previous || previous.spaceId !== voice.spaceId || previous.roomId !== voice.roomId)) {
    state.voiceSince = Date.now();
  }
  state.voice = voice;
  Call.syncVoice(voice);
  if (!voice) {
    Debug.remove('call');
    state.roomFeedback = '';
    state.focusTile = null;
    closeVoicePopover();
  }
  if (!el.soundboard.hidden) {
    renderSoundboard();
  }
  render();
}

function handleStreams(snapshot) {
  const wasLive = outgoingLive();
  state.streams = snapshot;
  const live = outgoingLive();
  StreamView.sync(snapshot.incoming, live);
  if (live && !wasLive) {
    Call.playLiveTone();
    startThumbLoop();
  } else if (!live && wasLive) {
    stopThumbLoop();
    StreamView.close(StreamView.SELF);
  }
  applyStreamVolumes();
  render();
}

function handleChatMessage({ spaceId, channelId, message }) {
  const key = conversationKey(spaceId, channelId);
  const shown = state.nav.view === 'chat' && Chat.receive(key, message);
  if (!shown) {
    state.unread.set(key, (state.unread.get(key) ?? 0) + 1);
  }
  renderRail();
  renderSidebar();
}

function handleSignaling(status) {
  state.signaling = status;
  refreshConversation();
  render();
}

function closeTopDialog() {
  if (AvatarEditor.isOpen()) {
    return;
  }
  if (!el.voicePopover.hidden) {
    closeVoicePopover();
  } else if (!el.contextMenu.hidden) {
    closeContextMenu();
  } else if (!el.soundboard.hidden && el.promptDialog.hidden) {
    closeSoundboard();
  } else if (!el.tileViewer.hidden) {
    closeTileViewer();
  } else if (!el.promptDialog.hidden) {
    closePrompt();
  } else if (!el.channelDialog.hidden) {
    closeChannelDialog();
  } else if (!el.settingsDialog.hidden) {
    closeSettings();
  } else if (!el.inviteDialog.hidden) {
    el.inviteDialog.hidden = true;
  } else if (!el.serverDialog.hidden) {
    el.serverDialog.hidden = true;
  } else if (Debug.isOpen()) {
    Debug.close();
  } else if (!el.serverMenu.hidden) {
    el.serverMenu.hidden = true;
  }
}

function bindEvents() {
  el.railHome.addEventListener('click', openHome);
  el.railAdd.addEventListener('click', openServerDialog);
  el.newConnection.addEventListener('click', openHome);
  el.chooseShare.addEventListener('click', startManualShare);
  el.chooseWatch.addEventListener('click', startManualWatch);
  el.chooseServer.addEventListener('click', openServerDialog);

  el.serverHeader.addEventListener('click', () => {
    el.serverMenu.hidden = !el.serverMenu.hidden;
  });
  for (const button of el.serverMenu.querySelectorAll('[data-server-action]')) {
    button.addEventListener('click', () => serverAction(button.dataset.serverAction));
  }
  for (const button of document.querySelectorAll('[data-add-channel]')) {
    button.addEventListener('click', () => {
      const server = currentServer();
      if (server) {
        openChannelDialog(server.id, null, button.dataset.addChannel);
      }
    });
  }
  document.addEventListener('click', (event) => {
    if (!el.serverMenu.hidden && !el.serverMenu.contains(event.target) && !el.serverHeader.contains(event.target)) {
      el.serverMenu.hidden = true;
    }
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      closeTopDialog();
    }
  });

  el.callStatus.addEventListener('click', toggleVoicePopover);
  el.callSounds.addEventListener('click', toggleSoundboard);
  el.callDebug.addEventListener('click', callPanelDebug);
  el.callLeave.addEventListener('click', callPanelLeave);
  el.callOpen.addEventListener('click', callPanelOpen);
  el.callCamera.addEventListener('click', toggleCamera);
  el.callShare.addEventListener('click', callPanelShare);
  el.callSettings.addEventListener('click', callPanelSettings);
  el.callFullscreen.addEventListener('click', toggleManualFullscreen);

  el.userButton.addEventListener('click', openSettings);
  el.userButton.addEventListener('contextmenu', (event) => {
    event.preventDefault();
    const items = [
      { type: 'header', label: state.profile.name },
      { type: 'action', label: 'Mudar avatar', onSelect: openAvatarPicker },
    ];
    if (state.profile.avatar) {
      items.push({ type: 'action', label: 'Remover avatar', danger: true, onSelect: removeAvatar });
    }
    openContextMenu(event, items);
  });
  el.settingsAvatarChange.addEventListener('click', openAvatarPicker);
  el.settingsAvatarRemove.addEventListener('click', removeAvatar);
  el.userSettings.addEventListener('click', openSettings);
  el.userMic.addEventListener('click', toggleMic);
  el.userDeaf.addEventListener('click', toggleDeaf);

  el.roomMic.addEventListener('click', toggleMic);
  el.roomCamera.addEventListener('click', toggleCamera);
  el.roomShare.addEventListener('click', toggleLive);
  el.roomSwitch.addEventListener('click', openOutgoingSwitch);
  el.roomSettings.addEventListener('click', openOutgoingSettings);
  el.roomLeave.addEventListener('click', leaveRoom);
  el.lobbyJoin.addEventListener('click', () => {
    if (state.lobbyRoom) {
      joinRoom(state.lobbyRoom.spaceId, state.lobbyRoom.roomId);
    }
  });

  el.copyCode.addEventListener('click', copyOutgoingCode);
  el.pasteCode.addEventListener('click', pasteFromClipboard);
  el.submitCode.addEventListener('click', submitCode);
  el.codeIn.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      submitCode();
    }
  });
  el.stageSwitch.addEventListener('click', manualSwitch);
  el.stageSettings.addEventListener('click', manualSettings);
  el.stageFullscreen.addEventListener('click', toggleManualFullscreen);
  el.stageDebug.addEventListener('click', () => Debug.open('manual'));
  el.stageStop.addEventListener('click', stopManual);
  el.stageDismiss.addEventListener('click', dismissManual);
  el.stageOpenChat.addEventListener('click', () => {
    const saved = state.manual && state.manual.saved;
    if (saved) {
      openContact(saved.contact.id);
    }
  });
  el.stageRename.addEventListener('click', () => {
    const saved = state.manual && state.manual.saved;
    const contact = saved ? findContact(saved.contact.id) : null;
    if (contact) {
      openContactMenu(contact);
    }
  });

  el.callDialogAccept.addEventListener('click', acceptIncomingCall);
  el.callDialogDecline.addEventListener('click', closeIncomingCall);

  el.promptSave.addEventListener('click', confirmPrompt);
  el.promptCancel.addEventListener('click', closePrompt);
  el.promptDanger.addEventListener('click', dangerPrompt);
  el.promptInput.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      confirmPrompt();
    }
  });

  el.serverDialogClose.addEventListener('click', () => {
    el.serverDialog.hidden = true;
  });
  el.serverCreate.addEventListener('click', createServer);
  el.serverCreateName.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      createServer();
    }
  });
  el.serverJoin.addEventListener('click', () => joinServer());
  el.serverJoinPaste.addEventListener('click', async () => {
    el.serverJoinCode.value = (await api.readClipboard()).trim();
    el.serverDialogError.textContent = '';
  });

  el.inviteClose.addEventListener('click', () => {
    el.inviteDialog.hidden = true;
  });
  el.inviteCopy.addEventListener('click', copyInvite);

  el.channelClose.addEventListener('click', closeChannelDialog);
  el.channelCancel.addEventListener('click', closeChannelDialog);
  el.channelSave.addEventListener('click', saveChannel);
  el.channelRemove.addEventListener('click', removeChannel);
  el.channelName.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      saveChannel();
    }
  });
  for (const input of el.channelKind.querySelectorAll('input[name="channel-kind"]')) {
    input.addEventListener('change', () => {
      el.channelName.placeholder = input.value === 'voice' ? 'Nova sala' : 'novo-canal';
    });
  }

  el.settingsClose.addEventListener('click', closeSettings);
  el.settingsNameSave.addEventListener('click', saveName);
  el.settingsName.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      saveName();
    }
  });
  el.settingsMic.addEventListener('change', async () => {
    await Call.setDevice('mic', el.settingsMic.value);
    await restartMicTest();
  });
  el.settingsSpeaker.addEventListener('change', () => {
    Call.setDevice('speaker', el.settingsSpeaker.value);
    StreamView.setSink(el.settingsSpeaker.value);
  });
  el.settingsCamera.addEventListener('change', async () => {
    await Call.setDevice('camera', el.settingsCamera.value);
    if (state.previewing) {
      stopSettingsPreview();
      await toggleSettingsPreview();
    }
  });
  el.settingsPreviewToggle.addEventListener('click', toggleSettingsPreview);
  el.settingsBlur.addEventListener('change', () => Call.setBlur(el.settingsBlur.checked));

  document.addEventListener('mousedown', (event) => {
    const inMenu = el.contextMenu.contains(event.target);
    if (!el.contextMenu.hidden && !inMenu) {
      closeContextMenu();
    }
    if (!el.voicePopover.hidden && !el.voicePopover.contains(event.target) && !el.callStatus.contains(event.target)) {
      closeVoicePopover();
    }
    if (!el.soundboard.hidden && !el.soundboard.contains(event.target) && !el.callSounds.contains(event.target)
      && !inMenu && el.promptDialog.hidden) {
      closeSoundboard();
    }
  }, true);
  document.addEventListener('scroll', (event) => {
    if (!el.contextMenu.hidden && !el.contextMenu.contains(event.target)) {
      closeContextMenu();
    }
  }, true);
  window.addEventListener('blur', closeContextMenu);
  window.addEventListener('resize', closeContextMenu);
  el.tileViewer.addEventListener('click', closeTileViewer);
  document.addEventListener('fullscreenchange', () => {
    if (!document.fullscreenElement && !el.tileViewer.hidden) {
      closeTileViewer();
    }
  });
  bindStageResize();
  bindVoicePopover();
  bindSoundboard();
}

function subscribeEvents() {
  api.onEvent(handleManualEvent);
  api.onLog((line) => Debug.appendLog(line));
  api.onExit(handleManualExit);
  api.onSessionStarted(handleSessionStarted);
  api.onContactSaved(handleContactSaved);
  api.onContacts(handleContacts);
  api.onServers(handleServers);
  api.onProfile((profile) => {
    state.profile = profile;
    renderSettingsAvatar();
    refreshConversation();
    render();
  });
  api.onPresence(handlePresence);
  api.onVoice(handleVoice);
  api.onRtc((message) => Call.handleRtc(message));
  api.onIncomingCall(handleIncomingCall);
  api.onChatMessage(handleChatMessage);
  api.onChatUpdate(({ spaceId, channelId, message }) => Chat.update(conversationKey(spaceId, channelId), message));
  api.onUploadProgress((info) => Chat.uploadProgress(info));
  api.onFileProgress(({
    spaceId, channelId, fileId, direction, done, total,
  }) => Chat.transferProgress({
    key: conversationKey(spaceId, channelId), fileId, direction, done, total, peers: 1,
  }));
  api.onFileStatus((info) => Chat.fetchStatus(info));
  FileTransfer.onProgress((info) => Chat.transferProgress(info));
  api.onSignalingStatus(handleSignaling);
  api.onStreams(handleStreams);
  api.onStreamEvent(handleStreamEvent);
  api.onStreamLog(({ source, line }) => {
    const label = source === 'outgoing' ? 'transmissão' : 'assistindo';
    Debug.appendLog(`[${label}] ${line}`);
  });
  api.onOutgoingEnded(handleOutgoingEnded);
  api.onIncomingEnded(handleIncomingEnded);
  api.onStreamOffer((payload) => StreamView.handleOffer(payload));
  api.onStreamThumb(handleStreamThumb);
  api.onAvatars(({ memberId, contactId, url }) => {
    if (contactId) {
      state.contactMembers.set(contactId, memberId);
    }
    if (url) {
      state.avatars.set(memberId, url);
    } else if (!contactId) {
      state.avatars.delete(memberId);
    }
    refreshConversation();
    render();
  });
  Call.subscribe(() => {
    StreamView.refreshAudio();
    render();
  });
  StreamView.subscribe(render);
  StreamView.onStats(handleStreamViewStats);
  Call.onStats(handleCallStats);
}

async function init() {
  state.profile = await api.profile();
  Call.init(state.profile.instanceId);
  Chat.bind({ profileName: () => state.profile.name });
  FileTransfer.bind();
  StreamView.configure({
    volumeFor: streamVolumeFor,
    deafFor: () => Call.state().deaf,
    sinkId: Call.state().settings.speakerId,
  });
  setInterval(updateVoiceTimers, 1000);
  bindEvents();
  subscribeEvents();

  const [contacts, servers, presence, status, streams, sounds, avatarSnapshot] = await Promise.all([
    api.listContacts(),
    api.listServers(),
    api.presenceSnapshot(),
    api.signalingStatus(),
    api.streamsState(),
    api.listSounds(),
    api.avatarsSnapshot(),
  ]);
  for (const [memberId, url] of Object.entries(avatarSnapshot.members)) {
    state.avatars.set(memberId, url);
  }
  for (const [contactId, memberId] of Object.entries(avatarSnapshot.contacts)) {
    state.contactMembers.set(contactId, memberId);
  }
  state.contacts = contacts;
  state.servers = servers;
  state.signaling = status;
  state.streams = streams;
  state.sounds = sounds;
  for (const [spaceId, peers] of Object.entries(presence)) {
    state.presence.set(spaceId, peers);
    Call.setPresence(spaceId, peers);
  }
  render();
}

init();
