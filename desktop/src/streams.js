'use strict';

const { EventEmitter } = require('node:events');
const { TelinhaSession, buildShareArgs, buildWatchArgs } = require('./telinha-process');

const WATCH_TIMEOUT_MS = 45000;
const VIEWER_GRACE_MS = 15000;
const RESTART_GRACE_MS = 8000;
const MAX_SDP_LENGTH = 60000;
const MAX_THUMB_LENGTH = 120000;
const SELF_VIEWER = 'self';

function asciiTitle(text) {
  return String(text ?? '')
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/[^\x20-\x7e]/g, '')
    .trim()
    .slice(0, 80);
}

class StreamManager extends EventEmitter {
  constructor({ exe, publish, selfId, ownerPid }) {
    super();
    this.exe = exe;
    this.publish = publish;
    this.selfId = selfId;
    this.ownerPid = ownerPid;
    this.outgoing = null;
    this.incoming = new Map();
  }

  snapshot() {
    const { outgoing } = this;
    return {
      outgoing: outgoing ? {
        spaceId: outgoing.spaceId,
        roomId: outgoing.roomId,
        state: outgoing.state,
        width: outgoing.width,
        height: outgoing.height,
        targetName: outgoing.targetName,
        viewers: [...outgoing.viewers]
          .filter(([id]) => id !== SELF_VIEWER)
          .map(([id, viewer]) => ({ id, state: viewer.state })),
      } : null,
      incoming: [...this.incoming.values()].map((entry) => ({
        sharerId: entry.sharerId,
        spaceId: entry.spaceId,
        roomId: entry.roomId,
        name: entry.name,
        mode: entry.mode,
        state: entry.state,
        fullscreen: entry.fullscreen,
      })),
    };
  }

  changed() {
    this.emit('changed', this.snapshot());
  }

  /* transmissao propria */

  goLive({ spaceId, roomId, share, targetName }) {
    if (this.outgoing) {
      throw new Error('voce ja esta transmitindo');
    }
    const args = buildShareArgs({ ...share, multi: true, excludePid: this.ownerPid });
    const session = new TelinhaSession(this.exe(), args);
    const outgoing = {
      session,
      spaceId,
      roomId,
      state: 'starting',
      width: 0,
      height: 0,
      targetName,
      error: null,
      viewers: new Map(),
    };
    this.outgoing = outgoing;

    session.on('event', (event) => this.handleOutgoingEvent(outgoing, event));
    session.on('log', (line) => this.emit('log', { source: 'outgoing', line }));
    session.on('exit', (info) => {
      if (this.outgoing !== outgoing) {
        return;
      }
      this.outgoing = null;
      this.publish(spaceId, 'stream-end', { roomId, reason: 'stopped' });
      this.emit('outgoing-ended', {
        code: info.code, stopped: info.stopped, error: outgoing.error, spaceId, roomId,
      });
      this.changed();
    });
    session.start();
    this.changed();
  }

  handleOutgoingEvent(outgoing, event) {
    this.emit('event', { source: 'outgoing', event });
    const viewer = event.peer ? outgoing.viewers.get(event.peer) : null;
    switch (event.event) {
      case 'ready':
        outgoing.state = 'live';
        outgoing.width = event.width;
        outgoing.height = event.height;
        this.watchOwnScreen(outgoing);
        this.changed();
        break;
      case 'target':
        outgoing.width = event.width;
        outgoing.height = event.height;
        this.changed();
        break;
      case 'code':
        if (event.kind === 'invite' && viewer) {
          viewer.state = 'inviting';
          this.publish(outgoing.spaceId, 'stream-offer', {
            to: event.peer, roomId: outgoing.roomId, code: event.code,
          });
          this.changed();
        }
        break;
      case 'offer':
        if (viewer && typeof event.sdp === 'string') {
          viewer.state = 'inviting';
          if (event.peer === SELF_VIEWER) {
            this.emit('embedded-offer', { sharerId: SELF_VIEWER, sdp: event.sdp });
          } else {
            this.publish(outgoing.spaceId, 'stream-offer', {
              to: event.peer, roomId: outgoing.roomId, sdp: event.sdp,
            });
          }
          this.changed();
        }
        break;
      case 'code_rejected':
        if (viewer) {
          outgoing.session.sendCommand({ command: 'remove_peer', peer: event.peer });
          if (event.peer !== SELF_VIEWER) {
            this.publish(outgoing.spaceId, 'stream-end', { to: event.peer, roomId: outgoing.roomId, reason: 'peer' });
          }
        }
        break;
      case 'peer_state':
        if (viewer) {
          viewer.state = event.state;
          this.changed();
        }
        break;
      case 'peer_failed':
        if (viewer && event.peer !== SELF_VIEWER) {
          this.publish(outgoing.spaceId, 'stream-end', { to: event.peer, roomId: outgoing.roomId, reason: 'peer' });
        }
        break;
      case 'peer_removed':
        if (outgoing.viewers.delete(event.peer)) {
          this.changed();
        }
        break;
      case 'error':
        outgoing.error = event;
        break;
      default:
        break;
    }
  }

  stopLive() {
    if (this.outgoing) {
      this.outgoing.session.stop();
    }
  }

  command(command) {
    const { outgoing } = this;
    return Boolean(outgoing && outgoing.state === 'live' && outgoing.session.sendCommand(command));
  }

  // O proprio emissor entra como espectador local, e assim ve a tela que esta mandando.
  watchOwnScreen(outgoing) {
    if (outgoing.viewers.has(SELF_VIEWER)) {
      return;
    }
    outgoing.viewers.set(SELF_VIEWER, { state: 'preparing', addedAt: Date.now(), format: 'sdp' });
    outgoing.session.sendCommand({ command: 'add_peer', peer: SELF_VIEWER, format: 'sdp' });
  }

  answerOwnScreen(sdp) {
    const { outgoing } = this;
    if (!outgoing || typeof sdp !== 'string' || sdp.length > MAX_SDP_LENGTH) {
      return false;
    }
    return outgoing.session.sendCommand({ command: 'peer_sdp_answer', peer: SELF_VIEWER, sdp });
  }

  publishThumb(image) {
    const { outgoing } = this;
    if (!outgoing || outgoing.state !== 'live' || typeof image !== 'string'
      || !image.startsWith('data:image/jpeg;base64,') || image.length > MAX_THUMB_LENGTH) {
      return false;
    }
    return this.publish(outgoing.spaceId, 'stream-thumb', { roomId: outgoing.roomId, image });
  }

  /* assistir */

  watch({ spaceId, roomId, sharerId, name, mode = 'embedded' }) {
    if (sharerId === this.selfId) {
      return;
    }
    const existing = this.incoming.get(sharerId);
    if (existing) {
      if (existing.mode !== mode) {
        this.switchMode(existing, mode);
      }
      return;
    }
    const entry = {
      session: null,
      spaceId,
      roomId,
      sharerId,
      name,
      mode,
      state: 'requesting',
      code: null,
      wantsCode: false,
      fullscreen: false,
      stopping: false,
      endedBySharer: false,
      switching: null,
      error: null,
      timer: null,
      requestedAt: 0,
    };
    this.incoming.set(sharerId, entry);
    try {
      this.requestStream(entry);
    } catch (error) {
      this.incoming.delete(sharerId);
      throw error;
    }
    this.changed();
  }

  requestStream(entry) {
    clearTimeout(entry.timer);
    entry.state = 'requesting';
    entry.requestedAt = Date.now();
    entry.error = null;
    entry.timer = setTimeout(() => {
      if (entry.state !== 'live' && this.incoming.get(entry.sharerId) === entry) {
        entry.error = { stage: 'timeout' };
        this.unwatch(entry.sharerId);
      }
    }, WATCH_TIMEOUT_MS);
    const format = entry.mode === 'embedded' ? 'sdp' : 'token';
    if (!this.publish(entry.spaceId, 'stream-watch', { to: entry.sharerId, roomId: entry.roomId, format })) {
      clearTimeout(entry.timer);
      throw new Error('sem conexao com o servidor de contatos');
    }
  }

  switchMode(entry, mode) {
    if (entry.session) {
      entry.switching = mode;
      entry.session.stop();
      return;
    }
    entry.mode = mode;
    try {
      this.requestStream(entry);
    } catch {
      this.drop(entry);
      return;
    }
    this.changed();
  }

  drop(entry) {
    clearTimeout(entry.timer);
    if (this.incoming.get(entry.sharerId) === entry) {
      this.incoming.delete(entry.sharerId);
    }
    if (!entry.endedBySharer) {
      this.publish(entry.spaceId, 'stream-leave', { to: entry.sharerId, roomId: entry.roomId });
    }
    let reason = 'stopped';
    if (entry.endedBySharer) {
      reason = 'ended';
    } else if (entry.error) {
      reason = 'failed';
    }
    this.emit('incoming-ended', {
      sharerId: entry.sharerId, name: entry.name, reason, error: entry.error, code: null,
    });
    this.changed();
  }

  startViewer(entry) {
    const session = new TelinhaSession(this.exe(), buildWatchArgs({ title: asciiTitle(`Telinha - ${entry.name}`) }));
    entry.session = session;
    entry.state = 'connecting';

    session.on('event', (event) => this.handleIncomingEvent(entry, event));
    session.on('log', (line) => this.emit('log', { source: entry.sharerId, line }));
    session.on('exit', (info) => {
      clearTimeout(entry.timer);
      entry.session = null;
      entry.code = null;
      entry.wantsCode = false;
      entry.fullscreen = false;

      const current = this.incoming.get(entry.sharerId) === entry;
      const closedByUser = !entry.stopping && !entry.endedBySharer && !entry.error && info.code === 0;
      const nextMode = entry.switching ?? (closedByUser ? 'embedded' : null);
      entry.switching = null;
      if (current && nextMode) {
        entry.stopping = false;
        entry.mode = nextMode;
        try {
          this.requestStream(entry);
          this.changed();
          return;
        } catch {
          // sem servidor de contatos, encerra abaixo
        }
      }

      let reason = 'closed';
      if (entry.endedBySharer) {
        reason = 'ended';
      } else if (entry.error) {
        reason = 'failed';
      } else if (entry.stopping) {
        reason = 'stopped';
      } else if (info.code !== 0) {
        reason = 'failed';
      }
      if (current) {
        this.incoming.delete(entry.sharerId);
      }
      if (!entry.endedBySharer) {
        this.publish(entry.spaceId, 'stream-leave', { to: entry.sharerId, roomId: entry.roomId });
      }
      this.emit('incoming-ended', {
        sharerId: entry.sharerId, name: entry.name, reason, error: entry.error, code: info.code,
      });
      this.changed();
    });
    session.start();
    this.changed();
  }

  handleIncomingEvent(entry, event) {
    this.emit('event', { source: entry.sharerId, event });
    switch (event.event) {
      case 'need_code':
        entry.wantsCode = true;
        this.deliverCode(entry);
        break;
      case 'code':
        if (event.kind === 'answer') {
          this.publish(entry.spaceId, 'stream-answer', {
            to: entry.sharerId, roomId: entry.roomId, code: event.code,
          });
        }
        break;
      case 'code_rejected':
        entry.error = { stage: 'code', message: event.message };
        if (entry.session) {
          entry.session.stop();
        }
        break;
      case 'state':
        if (event.state === 'Connected' && entry.state !== 'live') {
          entry.state = 'live';
          clearTimeout(entry.timer);
          this.changed();
        }
        break;
      case 'fullscreen':
        entry.fullscreen = Boolean(event.enabled);
        this.changed();
        break;
      case 'error':
        entry.error = event;
        break;
      default:
        break;
    }
  }

  deliverCode(entry) {
    if (entry.wantsCode && entry.code && entry.session) {
      const { code } = entry;
      entry.code = null;
      entry.wantsCode = false;
      entry.session.submitCode(code);
    }
  }

  unwatch(sharerId) {
    const entry = this.incoming.get(sharerId);
    if (!entry) {
      return;
    }
    entry.stopping = true;
    entry.switching = null;
    if (entry.session) {
      entry.session.stop();
      return;
    }
    this.drop(entry);
  }

  answerEmbedded(sharerId, sdp) {
    if (sharerId === SELF_VIEWER) {
      return this.answerOwnScreen(sdp);
    }
    const entry = this.incoming.get(sharerId);
    if (!entry || entry.mode !== 'embedded' || typeof sdp !== 'string' || sdp.length > MAX_SDP_LENGTH) {
      return false;
    }
    return this.publish(entry.spaceId, 'stream-answer', { to: sharerId, roomId: entry.roomId, sdp });
  }

  embeddedState(sharerId, state) {
    const entry = this.incoming.get(sharerId);
    if (!entry || entry.mode !== 'embedded') {
      return;
    }
    if (state === 'live') {
      if (entry.state !== 'live') {
        entry.state = 'live';
        clearTimeout(entry.timer);
        this.changed();
      }
    } else if (state === 'failed') {
      entry.error = { stage: 'network' };
      this.unwatch(sharerId);
    }
  }

  setVolume(sharerId, volume) {
    const entry = this.incoming.get(sharerId);
    const value = Math.max(0, Math.min(200, Math.round(Number(volume) || 0)));
    return Boolean(entry && entry.session && entry.state === 'live'
      && entry.session.sendCommand({ command: 'set_volume', volume: value }));
  }

  setFullscreen(sharerId, enabled) {
    const entry = this.incoming.get(sharerId);
    return Boolean(entry && entry.session && entry.state === 'live'
      && entry.session.sendCommand({ command: 'set_fullscreen', enabled: Boolean(enabled) }));
  }

  /* mensagens */

  handleMessage(spaceId, message) {
    const { from } = message;
    if (message.to && message.to !== this.selfId) {
      return;
    }

    if (message.type === 'stream-watch') {
      const { outgoing } = this;
      if (!outgoing || outgoing.state !== 'live' || outgoing.spaceId !== spaceId
        || outgoing.roomId !== message.roomId) {
        this.publish(spaceId, 'stream-end', { to: from, roomId: message.roomId, reason: 'stopped' });
        return;
      }
      const format = message.format === 'sdp' ? 'sdp' : 'token';
      outgoing.viewers.set(from, { state: 'preparing', addedAt: Date.now(), format });
      outgoing.session.sendCommand({ command: 'add_peer', peer: from, format });
      this.changed();
      return;
    }

    if (message.type === 'stream-offer') {
      const entry = this.incoming.get(from);
      if (!entry || entry.spaceId !== spaceId || entry.stopping) {
        return;
      }
      if (entry.mode === 'embedded') {
        if (typeof message.sdp !== 'string' || message.sdp.length > MAX_SDP_LENGTH) {
          return;
        }
        entry.state = 'connecting';
        this.emit('embedded-offer', {
          sharerId: from, spaceId, roomId: entry.roomId, name: entry.name, sdp: message.sdp,
        });
        this.changed();
        return;
      }
      if (typeof message.code !== 'string') {
        return;
      }
      entry.code = message.code;
      if (!entry.session) {
        this.startViewer(entry);
      }
      this.deliverCode(entry);
      return;
    }

    if (message.type === 'stream-answer') {
      const { outgoing } = this;
      const viewer = outgoing && outgoing.spaceId === spaceId ? outgoing.viewers.get(from) : null;
      if (!viewer) {
        return;
      }
      if (typeof message.sdp === 'string' && message.sdp.length <= MAX_SDP_LENGTH) {
        outgoing.session.sendCommand({ command: 'peer_sdp_answer', peer: from, sdp: message.sdp });
      } else if (typeof message.code === 'string') {
        outgoing.session.sendCommand({ command: 'peer_answer', peer: from, code: message.code });
      } else {
        return;
      }
      viewer.state = 'connecting';
      this.changed();
      return;
    }

    if (message.type === 'stream-leave') {
      const { outgoing } = this;
      if (outgoing && outgoing.spaceId === spaceId && outgoing.viewers.has(from)) {
        outgoing.session.sendCommand({ command: 'remove_peer', peer: from });
      }
      return;
    }

    if (message.type === 'stream-thumb') {
      if (typeof message.image !== 'string' || message.image.length > MAX_THUMB_LENGTH
        || !message.image.startsWith('data:image/jpeg;base64,')) {
        return;
      }
      this.emit('thumb', {
        sharerId: from, spaceId, roomId: message.roomId, image: message.image,
      });
      return;
    }

    if (message.type === 'stream-end') {
      const entry = this.incoming.get(from);
      if (!entry || entry.spaceId !== spaceId) {
        return;
      }
      if (message.reason === 'peer' && Date.now() - entry.requestedAt < RESTART_GRACE_MS) {
        return;
      }
      entry.endedBySharer = true;
      this.unwatch(from);
    }
  }

  reconcile(spaceId, roomId, peers) {
    const inRoom = new Map(peers
      .filter((peer) => peer.voice && peer.voice.roomId === roomId)
      .map((peer) => [peer.id, peer]));

    for (const entry of [...this.incoming.values()]) {
      if (entry.spaceId !== spaceId) {
        continue;
      }
      const sharer = inRoom.get(entry.sharerId);
      if (!sharer || !sharer.voice.live) {
        entry.endedBySharer = true;
        this.unwatch(entry.sharerId);
      }
    }

    const { outgoing } = this;
    if (outgoing && outgoing.spaceId === spaceId) {
      const now = Date.now();
      for (const [viewerId, viewer] of outgoing.viewers) {
        if (viewerId !== SELF_VIEWER && !inRoom.has(viewerId) && now - viewer.addedAt > VIEWER_GRACE_MS) {
          outgoing.session.sendCommand({ command: 'remove_peer', peer: viewerId });
        }
      }
    }
  }

  leaveRoom() {
    this.stopLive();
    for (const sharerId of [...this.incoming.keys()]) {
      this.unwatch(sharerId);
    }
  }

  stopAll() {
    this.leaveRoom();
  }
}

module.exports = { SELF_VIEWER, StreamManager, asciiTitle };
