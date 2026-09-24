'use strict';

const Call = (() => {
  const api = window.telinha;
  const ICE_SERVERS = [{ urls: ['stun:stun.l.google.com:19302', 'stun:stun.cloudflare.com:3478'] }];
  const SPEAKING_LEVEL = 0.03;
  const SPEAKING_HOLD_MS = 350;
  const RETRY_DELAY_MS = 3000;
  const STATS_INTERVAL_MS = 2000;
  const PEER_GRACE_MS = 5000;
  const SETTINGS_KEY = 'telinha.devices';
  const PEOPLE_KEY = 'telinha.people';

  const ICONS = {
    micOff: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3a3 3 0 0 0-3 3v5a3 3 0 0 0 5.2 2M15 10V6a3 3 0 0 0-5.6-1.5M5 11a7 7 0 0 0 11.5 5.4M19 11a7 7 0 0 1-.6 2.8M12 18v3M4 4l16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
    deaf: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 15v-3a8 8 0 0 1 13.7-5.6M20 12v3M4 15h3v5H5a1 1 0 0 1-1-1zM17 15h3v4a1 1 0 0 1-1 1h-2zM4 4l16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  };

  const EXPAND_ICON = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>';

  const POPOUT_ICON = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M14 4h6v6M20 4l-8 8M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>';
  const CLOSE_ICON = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/></svg>';
  const AVATAR_COLORS = ['#5865f2', '#3ba55c', '#faa61a', '#ed4245', '#eb459e', '#9b59b6', '#1abc9c', '#e67e22'];
  const MAX_SOUND_SECONDS = 10;
  const SOUND_VOLUME_KEY = 'telinha.soundVolume';

  const listeners = new Set();
  const statsListeners = new Set();
  const peers = new Map();
  const presence = new Map();
  const settings = loadSettings();
  const people = loadPeople();
  const local = {
    mic: null,
    camera: null,
    micOn: settings.micOn,
    cameraOn: false,
    deaf: settings.deaf,
    micError: null,
    cameraError: null,
    meter: null,
    speakingUntil: 0,
    videoEl: null,
  };

  let selfId = '';
  let room = null;
  let busy = false;
  let audioContext = null;
  let meterTimer = null;
  let statsTimer = null;
  let previewTrack = null;
  let testTrack = null;
  let testMeter = null;
  let testTimer = null;
  let knownParticipants = new Set();
  let knownLive = new Set();
  let soundVolume = loadSoundVolume();
  const mix = { destination: null, micSource: null, soundBus: null, track: null };
  const soundBuffers = new Map();

  function loadSettings() {
    const defaults = { micId: '', cameraId: '', speakerId: '', blur: false, micOn: true, deaf: false };
    try {
      const data = JSON.parse(localStorage.getItem(SETTINGS_KEY) || '{}');
      return {
        micId: typeof data.micId === 'string' ? data.micId : '',
        cameraId: typeof data.cameraId === 'string' ? data.cameraId : '',
        speakerId: typeof data.speakerId === 'string' ? data.speakerId : '',
        blur: data.blur === true,
        micOn: data.micOn !== false,
        deaf: data.deaf === true,
      };
    } catch {
      return defaults;
    }
  }

  function saveSettings() {
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
    } catch {
      // armazenamento local indisponivel, as escolhas valem so nesta execucao
    }
  }

  function emit() {
    for (const listener of listeners) {
      listener();
    }
  }

  function subscribe(listener) {
    listeners.add(listener);
    return () => listeners.delete(listener);
  }

  function onStats(listener) {
    statsListeners.add(listener);
    return () => statsListeners.delete(listener);
  }

  function initial(name) {
    return String(name || '?').charAt(0).toUpperCase();
  }

  /* ajustes por pessoa */

  function loadPeople() {
    try {
      const data = JSON.parse(localStorage.getItem(PEOPLE_KEY) || '{}');
      return data && typeof data === 'object' ? data : {};
    } catch {
      return {};
    }
  }

  function savePeople() {
    try {
      localStorage.setItem(PEOPLE_KEY, JSON.stringify(people));
    } catch {
      // sem armazenamento local, os ajustes valem so nesta execucao
    }
  }

  function prefsKey(participant) {
    if (!participant) {
      return '';
    }
    return participant.memberId ? String(participant.memberId) : String(participant.id);
  }

  function clampPercent(value, fallback) {
    return Number.isFinite(value) ? Math.max(0, Math.min(200, Math.round(value))) : fallback;
  }

  function personPrefs(key) {
    const stored = people[key] || {};
    return {
      volume: clampPercent(stored.volume, 100),
      streamVolume: clampPercent(stored.streamVolume, 100),
      muted: stored.muted === true,
      videoHidden: stored.videoHidden === true,
    };
  }

  function setPersonPrefs(key, changes) {
    if (!key) {
      return;
    }
    people[key] = { ...personPrefs(key), ...changes };
    savePeople();
    for (const peer of peers.values()) {
      applyPeerAudio(peer);
    }
    if ('videoHidden' in changes) {
      emit();
    }
  }

  function videoStream(id) {
    if (id === selfId) {
      return local.camera ? new MediaStream([local.camera]) : null;
    }
    const peer = peers.get(id);
    return peer && peer.videoTrack ? new MediaStream([peer.videoTrack]) : null;
  }

  /* audio local e medidores */

  function ensureAudioContext() {
    if (!audioContext) {
      audioContext = new AudioContext();
      if (settings.speakerId && typeof audioContext.setSinkId === 'function') {
        audioContext.setSinkId(settings.speakerId).catch(() => {});
      }
    }
    if (audioContext.state === 'suspended') {
      audioContext.resume().catch(() => {});
    }
    return audioContext;
  }

  function createMeter(track) {
    try {
      const context = ensureAudioContext();
      const source = context.createMediaStreamSource(new MediaStream([track]));
      const analyser = context.createAnalyser();
      analyser.fftSize = 512;
      source.connect(analyser);
      return { source, analyser, data: new Float32Array(analyser.fftSize) };
    } catch {
      return null;
    }
  }

  function meterLevel(meter) {
    if (!meter) {
      return 0;
    }
    meter.analyser.getFloatTimeDomainData(meter.data);
    let sum = 0;
    for (const value of meter.data) {
      sum += value * value;
    }
    return Math.sqrt(sum / meter.data.length);
  }

  function closeMeter(meter) {
    if (meter) {
      try {
        meter.source.disconnect();
      } catch {
        // o no ja foi desligado
      }
    }
  }

  function describeMediaError(error, device) {
    const name = error && error.name;
    if (name === 'NotAllowedError' || name === 'SecurityError') {
      return `O Windows bloqueou o acesso ${device}. Libere em Configurações > Privacidade e segurança.`;
    }
    if (name === 'NotFoundError' || name === 'OverconstrainedError') {
      return `Nenhum dispositivo encontrado para ${device}.`;
    }
    if (name === 'NotReadableError') {
      return `O dispositivo de ${device} está em uso por outro programa.`;
    }
    return `Não consegui abrir ${device}.`;
  }

  async function captureTrack(kind, deviceId) {
    const base = kind === 'audio'
      ? { echoCancellation: true, noiseSuppression: true, autoGainControl: true }
      : { width: { ideal: 1280 }, height: { ideal: 720 }, frameRate: { ideal: 30 } };
    const request = async (id) => {
      const stream = await navigator.mediaDevices.getUserMedia({
        [kind]: id ? { ...base, deviceId: { exact: id } } : base,
      });
      return stream.getTracks().find((track) => track.kind === kind);
    };
    try {
      return await request(deviceId);
    } catch (error) {
      if (!deviceId) {
        throw error;
      }
      return request('');
    }
  }

  async function openMic() {
    if (local.mic && local.mic.readyState === 'live') {
      return;
    }
    try {
      local.mic = await captureTrack('audio', settings.micId);
      local.mic.enabled = local.micOn && !local.deaf;
      local.micError = null;
      closeMeter(local.meter);
      local.meter = createMeter(local.mic);
      connectMic();
    } catch (error) {
      local.mic = null;
      disconnectMic();
      local.micError = describeMediaError(error, 'ao microfone');
    }
  }

  function stopLocalMedia() {
    closeMeter(local.meter);
    local.meter = null;
    disconnectMic();
    if (local.mic) {
      local.mic.stop();
      local.mic = null;
    }
    if (local.camera) {
      local.camera.stop();
      local.camera = null;
    }
    local.cameraOn = false;
    if (local.videoEl) {
      local.videoEl.srcObject = null;
    }
  }

  function micLive() {
    return Boolean(local.mic) && local.micOn && !local.deaf;
  }

  function applyTrack(kind) {
    for (const peer of peers.values()) {
      const transceiver = kind === 'audio' ? peer.audioTx : peer.videoTx;
      const track = kind === 'audio' ? outgoingAudio() : (local.cameraOn ? local.camera : null);
      if (transceiver && peer.pc && peer.pc.signalingState !== 'closed') {
        transceiver.sender.replaceTrack(track).catch(() => {});
      }
    }
  }

  /* sala */

  function participants() {
    if (!room) {
      return [];
    }
    return (presence.get(room.spaceId) || [])
      .filter((peer) => peer.voice && peer.voice.roomId === room.roomId && peer.id !== selfId);
  }

  function setPresence(spaceId, list) {
    presence.set(spaceId, Array.isArray(list) ? list : []);
    if (room && room.spaceId === spaceId) {
      reconcile();
    }
  }

  function roomPeople(spaceId, roomId) {
    return (presence.get(spaceId) || []).filter((peer) => peer.voice && peer.voice.roomId === roomId);
  }

  async function join(spaceId, roomId) {
    if (busy) {
      return false;
    }
    if (room && room.spaceId === spaceId && room.roomId === roomId) {
      return true;
    }
    busy = true;
    emit();
    try {
      if (room) {
        await leave();
      }
      await openMic();
      const state = await api.voiceJoin({
        spaceId, roomId, mic: micLive(), camera: false, deaf: local.deaf,
      });
      room = { spaceId: state.spaceId, roomId: state.roomId };
      knownParticipants = new Set(participants().map((peer) => peer.id));
      knownLive = new Set(participants().filter((peer) => peer.voice.live).map((peer) => peer.id));
      playTone('join');
      startTimers();
      reconcile();
      return true;
    } catch (error) {
      stopLocalMedia();
      throw error;
    } finally {
      busy = false;
      emit();
    }
  }

  function closeRoom() {
    if (room) {
      playTone('leave');
    }
    room = null;
    knownParticipants = new Set();
    knownLive = new Set();
    for (const peer of [...peers.values()]) {
      closePeer(peer);
    }
    stopTimers();
    stopLocalMedia();
  }

  async function leave() {
    if (!room) {
      return;
    }
    closeRoom();
    emit();
    await api.voiceLeave().catch(() => {});
  }

  function syncVoice(voice) {
    if (!voice && room && !busy) {
      closeRoom();
      emit();
    }
  }

  function currentRoom() {
    return room ? { ...room } : null;
  }

  function inRoom(spaceId, roomId) {
    return Boolean(room && room.spaceId === spaceId && room.roomId === roomId);
  }

  /* conexoes */

  function createPeer(id) {
    const peer = {
      id,
      initiator: selfId < id,
      session: null,
      pc: null,
      audioTx: null,
      videoTx: null,
      audioTrack: null,
      videoTrack: null,
      audioEl: null,
      videoEl: null,
      meter: null,
      pending: [],
      state: 'new',
      createdAt: Date.now(),
      retryTimer: null,
      speakingUntil: 0,
      stats: { rtt: null, loss: 0, lost: 0, received: 0 },
    };
    peers.set(id, peer);
    return peer;
  }

  function closePeer(peer) {
    clearTimeout(peer.retryTimer);
    if (peer.pc) {
      peer.pc.close();
      peer.pc = null;
    }
    closeMeter(peer.meter);
    peer.meter = null;
    closeBoost(peer);
    if (peer.keepAliveEl) {
      peer.keepAliveEl.srcObject = null;
      peer.keepAliveEl.remove();
    }
    if (peer.audioEl) {
      peer.audioEl.srcObject = null;
      peer.audioEl.remove();
    }
    if (peer.videoEl) {
      peer.videoEl.srcObject = null;
    }
    peers.delete(peer.id);
  }

  function reconcile() {
    if (!room) {
      return;
    }
    const desired = new Set(participants().map((peer) => peer.id));
    const arrived = [...desired].some((id) => !knownParticipants.has(id));
    const departed = [...knownParticipants].some((id) => !desired.has(id));
    knownParticipants = desired;
    if (arrived) {
      playTone('join');
    } else if (departed) {
      playTone('leave');
    }

    const live = new Set(participants().filter((peer) => peer.voice.live).map((peer) => peer.id));
    const started = [...live].some((id) => !knownLive.has(id));
    knownLive = live;
    if (started) {
      playTone('live');
    }
    const now = Date.now();
    for (const peer of [...peers.values()]) {
      if (!desired.has(peer.id) && now - peer.createdAt > PEER_GRACE_MS) {
        closePeer(peer);
      }
    }
    for (const id of desired) {
      if (!peers.has(id)) {
        const peer = createPeer(id);
        if (peer.initiator) {
          startOffer(peer);
        }
      }
    }
    emit();
  }

  function send(peer, payload) {
    if (room) {
      api.rtcSend(room.spaceId, peer.id, payload).catch(() => {});
    }
  }

  function newConnection(peer) {
    clearTimeout(peer.retryTimer);
    if (peer.pc) {
      peer.pc.close();
    }
    closeMeter(peer.meter);
    peer.meter = null;
    peer.audioTx = null;
    peer.videoTx = null;
    peer.audioTrack = null;
    peer.videoTrack = null;
    peer.stats = { rtt: null, loss: 0, lost: 0, received: 0 };

    const pc = new RTCPeerConnection({ iceServers: ICE_SERVERS });
    peer.pc = pc;
    peer.state = 'connecting';

    pc.onicecandidate = ({ candidate }) => {
      if (candidate && peer.pc === pc) {
        send(peer, { kind: 'candidate', session: peer.session, candidate: candidate.toJSON() });
      }
    };
    pc.ontrack = ({ track, transceiver }) => {
      if (peer.pc !== pc) {
        return;
      }
      if (track.kind === 'audio') {
        peer.audioTx = transceiver;
        attachAudio(peer, track);
      } else {
        peer.videoTx = transceiver;
        attachVideo(peer, track);
      }
      emit();
    };
    pc.onconnectionstatechange = () => {
      if (peer.pc !== pc) {
        return;
      }
      peer.state = pc.connectionState;
      if (pc.connectionState === 'failed') {
        scheduleRetry(peer);
      }
      emit();
    };
    return pc;
  }

  async function startOffer(peer) {
    const pc = newConnection(peer);
    peer.session = crypto.randomUUID();
    peer.pending = [];
    try {
      peer.audioTx = pc.addTransceiver('audio', { direction: 'sendrecv' });
      peer.videoTx = pc.addTransceiver('video', { direction: 'sendrecv' });
      await peer.audioTx.sender.replaceTrack(outgoingAudio());
      await peer.videoTx.sender.replaceTrack(local.cameraOn ? local.camera : null);
      await pc.setLocalDescription(await pc.createOffer());
      if (peer.pc === pc) {
        send(peer, { kind: 'offer', session: peer.session, description: pc.localDescription.toJSON() });
      }
    } catch {
      if (peer.pc === pc) {
        scheduleRetry(peer);
      }
    }
  }

  async function flushCandidates(peer, pc) {
    const ready = peer.pending.filter((item) => item.session === peer.session);
    peer.pending = [];
    for (const item of ready) {
      try {
        await pc.addIceCandidate(item.candidate);
      } catch {
        // candidato velho ou invalido, a conexao segue com os outros
      }
    }
  }

  async function acceptOffer(peer, payload) {
    const pc = newConnection(peer);
    peer.session = payload.session;
    try {
      await pc.setRemoteDescription(payload.description);
      for (const transceiver of pc.getTransceivers()) {
        transceiver.direction = 'sendrecv';
        if (transceiver.receiver.track.kind === 'audio') {
          peer.audioTx = transceiver;
          await transceiver.sender.replaceTrack(outgoingAudio());
        } else {
          peer.videoTx = transceiver;
          await transceiver.sender.replaceTrack(local.cameraOn ? local.camera : null);
        }
      }
      await flushCandidates(peer, pc);
      await pc.setLocalDescription(await pc.createAnswer());
      if (peer.pc === pc) {
        send(peer, { kind: 'answer', session: peer.session, description: pc.localDescription.toJSON() });
      }
    } catch {
      peer.state = 'failed';
      emit();
    }
  }

  function scheduleRetry(peer) {
    clearTimeout(peer.retryTimer);
    peer.state = 'failed';
    if (!peer.initiator) {
      return;
    }
    peer.retryTimer = setTimeout(() => {
      if (room && peers.get(peer.id) === peer && participants().some((item) => item.id === peer.id)) {
        startOffer(peer);
      }
    }, RETRY_DELAY_MS);
  }

  async function handleRtc({ spaceId, from, payload }) {
    if (!room || room.spaceId !== spaceId || !payload || typeof payload.session !== 'string'
      || typeof from !== 'string') {
      return;
    }
    const peer = peers.get(from) ?? createPeer(from);

    if (payload.kind === 'offer') {
      if (!peer.initiator && payload.description) {
        await acceptOffer(peer, payload);
      }
      return;
    }

    if (payload.kind === 'answer') {
      const { pc } = peer;
      if (!pc || peer.session !== payload.session || pc.signalingState !== 'have-local-offer') {
        return;
      }
      try {
        await pc.setRemoteDescription(payload.description);
        await flushCandidates(peer, pc);
      } catch {
        scheduleRetry(peer);
      }
      return;
    }

    if (payload.kind === 'candidate' && payload.candidate) {
      const { pc } = peer;
      if (pc && peer.session === payload.session && pc.remoteDescription) {
        try {
          await pc.addIceCandidate(payload.candidate);
        } catch {
          // candidato fora de ordem, ignorado
        }
        return;
      }
      peer.pending.push({ session: payload.session, candidate: payload.candidate });
      if (peer.pending.length > 100) {
        peer.pending.shift();
      }
    }
  }

  /* midia remota */

  function applySink(element) {
    if (settings.speakerId && typeof element.setSinkId === 'function') {
      element.setSinkId(settings.speakerId).catch(() => {});
    }
  }

  function closeBoost(peer) {
    if (peer.boost) {
      try {
        peer.boost.source.disconnect();
        peer.boost.gain.disconnect();
      } catch {
        // os nos ja estavam desligados
      }
      peer.boost = null;
    }
  }

  function applyPeerAudio(peer) {
    if (!peer.audioEl || !peer.directStream) {
      return;
    }
    const participant = participants().find((item) => item.id === peer.id);
    const prefs = personPrefs(prefsKey(participant || { id: peer.id }));
    const volume = prefs.muted ? 0 : prefs.volume;

    if (volume > 100 && !peer.boost) {
      try {
        const context = ensureAudioContext();
        const source = context.createMediaStreamSource(peer.directStream);
        const gain = context.createGain();
        const destination = context.createMediaStreamDestination();
        source.connect(gain);
        gain.connect(destination);
        peer.boost = { source, gain, destination };
      } catch {
        peer.boost = null;
      }
    }

    if (volume > 100 && peer.boost) {
      peer.boost.gain.gain.value = volume / 100;
      if (peer.audioEl.srcObject !== peer.boost.destination.stream) {
        peer.audioEl.srcObject = peer.boost.destination.stream;
      }
      peer.audioEl.volume = 1;
    } else {
      if (peer.audioEl.srcObject !== peer.directStream) {
        peer.audioEl.srcObject = peer.directStream;
      }
      peer.audioEl.volume = Math.min(1, volume / 100);
    }
    peer.audioEl.muted = local.deaf;
    peer.audioEl.play().catch(() => {});
  }

  function attachAudio(peer, track) {
    peer.audioTrack = track;
    peer.directStream = new MediaStream([track]);
    closeBoost(peer);
    if (!peer.keepAliveEl) {
      peer.keepAliveEl = document.createElement('audio');
      peer.keepAliveEl.muted = true;
      peer.keepAliveEl.hidden = true;
      document.body.append(peer.keepAliveEl);
    }
    peer.keepAliveEl.srcObject = peer.directStream;
    peer.keepAliveEl.play().catch(() => {});
    if (!peer.audioEl) {
      peer.audioEl = document.createElement('audio');
      peer.audioEl.autoplay = true;
      peer.audioEl.hidden = true;
      document.body.append(peer.audioEl);
    }
    applySink(peer.audioEl);
    applyPeerAudio(peer);
    closeMeter(peer.meter);
    peer.meter = createMeter(track);
  }

  function attachVideo(peer, track) {
    peer.videoTrack = track;
    if (!peer.videoEl) {
      peer.videoEl = document.createElement('video');
      peer.videoEl.autoplay = true;
      peer.videoEl.playsInline = true;
      peer.videoEl.muted = true;
    }
    peer.videoEl.srcObject = new MediaStream([track]);
    track.onmute = emit;
    track.onunmute = emit;
  }

  /* fala e estatisticas */

  function markSpeaking(id, speaking) {
    for (const element of document.querySelectorAll(`[data-voice-id="${id}"]`)) {
      element.classList.toggle('speaking', speaking);
    }
  }

  function updateSpeaking() {
    const now = performance.now();
    if (micLive() && meterLevel(local.meter) > SPEAKING_LEVEL) {
      local.speakingUntil = now + SPEAKING_HOLD_MS;
    }
    markSpeaking(selfId, now < local.speakingUntil);
    for (const peer of peers.values()) {
      if (peer.meter && !local.deaf && meterLevel(peer.meter) > SPEAKING_LEVEL) {
        peer.speakingUntil = now + SPEAKING_HOLD_MS;
      }
      markSpeaking(peer.id, now < peer.speakingUntil);
    }
  }

  async function collectStats() {
    const results = [];
    for (const peer of [...peers.values()]) {
      if (!peer.pc || peer.pc.connectionState !== 'connected') {
        continue;
      }
      try {
        const report = await peer.pc.getStats();
        let pair = null;
        let lost = 0;
        let received = 0;
        report.forEach((stat) => {
          if (stat.type === 'transport' && stat.selectedCandidatePairId) {
            pair = report.get(stat.selectedCandidatePairId) || pair;
          } else if (stat.type === 'inbound-rtp') {
            lost += stat.packetsLost || 0;
            received += stat.packetsReceived || 0;
          }
        });
        if (!pair) {
          report.forEach((stat) => {
            if (stat.type === 'candidate-pair' && stat.nominated && stat.state === 'succeeded') {
              pair = stat;
            }
          });
        }
        const deltaLost = Math.max(0, lost - peer.stats.lost);
        const deltaReceived = Math.max(0, received - peer.stats.received);
        peer.stats = {
          rtt: pair && Number.isFinite(pair.currentRoundTripTime) ? pair.currentRoundTripTime * 1000 : null,
          loss: deltaLost + deltaReceived > 0 ? deltaLost / (deltaLost + deltaReceived) : 0,
          lost,
          received,
          local: pair ? pair.localCandidateId : null,
        };
        results.push(peer.stats);
      } catch {
        // a conexao fechou no meio da leitura
      }
    }
    const rtts = results.map((item) => item.rtt).filter((value) => value !== null);
    const summary = {
      peers: peers.size,
      connected: results.length,
      rtt: rtts.length > 0 ? rtts.reduce((sum, value) => sum + value, 0) / rtts.length : null,
      loss: results.length > 0 ? Math.max(...results.map((item) => item.loss)) : null,
    };
    for (const listener of statsListeners) {
      listener(summary);
    }
  }

  function startTimers() {
    stopTimers();
    meterTimer = setInterval(updateSpeaking, 100);
    statsTimer = setInterval(collectStats, STATS_INTERVAL_MS);
  }

  function stopTimers() {
    clearInterval(meterTimer);
    clearInterval(statsTimer);
    meterTimer = null;
    statsTimer = null;
  }

  /* controles */

  async function setMicOn(value) {
    local.micOn = Boolean(value);
    if (local.micOn && local.deaf) {
      await setDeaf(false);
    }
    if (room && local.micOn && !local.mic) {
      await openMic();
      applyTrack('audio');
    }
    if (local.mic) {
      local.mic.enabled = micLive();
    }
    settings.micOn = local.micOn;
    saveSettings();
    if (room) {
      await api.voiceUpdate({ mic: micLive(), deaf: local.deaf }).catch(() => {});
    }
    emit();
  }

  async function setDeaf(value) {
    local.deaf = Boolean(value);
    for (const peer of peers.values()) {
      applyPeerAudio(peer);
    }
    if (local.mic) {
      local.mic.enabled = micLive();
    }
    settings.deaf = local.deaf;
    saveSettings();
    if (room) {
      await api.voiceUpdate({ mic: micLive(), deaf: local.deaf }).catch(() => {});
    }
    emit();
  }

  function blurSupported(track = local.camera) {
    try {
      const capabilities = track && track.getCapabilities ? track.getCapabilities() : null;
      return Boolean(capabilities && Array.isArray(capabilities.backgroundBlur)
        && capabilities.backgroundBlur.includes(true));
    } catch {
      return false;
    }
  }

  async function applyBlur(track) {
    if (track && blurSupported(track)) {
      await track.applyConstraints({ backgroundBlur: settings.blur }).catch(() => {});
    }
  }

  async function setCameraOn(value) {
    if (!room) {
      return;
    }
    if (value) {
      let track;
      try {
        track = await captureTrack('video', settings.cameraId);
      } catch (error) {
        local.cameraError = describeMediaError(error, 'à câmera');
        emit();
        throw new Error(local.cameraError);
      }
      if (!room) {
        track.stop();
        return;
      }
      if (local.camera) {
        local.camera.stop();
      }
      local.camera = track;
      local.cameraOn = true;
      local.cameraError = null;
      await applyBlur(track);
      track.onended = () => {
        if (local.camera === track) {
          setCameraOn(false);
        }
      };
    } else {
      local.cameraOn = false;
      if (local.camera) {
        local.camera.stop();
        local.camera = null;
      }
    }
    applyTrack('video');
    await api.voiceUpdate({ camera: local.cameraOn }).catch(() => {});
    emit();
  }

  async function setDevice(kind, deviceId) {
    const id = String(deviceId || '');
    if (kind === 'speaker') {
      settings.speakerId = id;
      if (audioContext && typeof audioContext.setSinkId === 'function') {
        audioContext.setSinkId(id).catch(() => {});
      }
      for (const peer of peers.values()) {
        if (peer.audioEl) {
          applySink(peer.audioEl);
        }
      }
    } else if (kind === 'mic') {
      settings.micId = id;
      if (room) {
        const old = local.mic;
        local.mic = null;
        closeMeter(local.meter);
        local.meter = null;
        await openMic();
        applyTrack('audio');
        if (old) {
          old.stop();
        }
      }
    } else if (kind === 'camera') {
      settings.cameraId = id;
      if (room && local.cameraOn) {
        await setCameraOn(true).catch(() => {});
      }
    }
    saveSettings();
    emit();
  }

  async function setBlur(value) {
    settings.blur = Boolean(value);
    saveSettings();
    await applyBlur(local.camera);
    await applyBlur(previewTrack);
    emit();
  }

  async function listDevices() {
    try {
      const devices = await navigator.mediaDevices.enumerateDevices();
      return {
        mics: devices.filter((device) => device.kind === 'audioinput'),
        cameras: devices.filter((device) => device.kind === 'videoinput'),
        speakers: devices.filter((device) => device.kind === 'audiooutput'),
      };
    } catch {
      return { mics: [], cameras: [], speakers: [] };
    }
  }

  async function startPreview(videoElement) {
    stopPreview();
    try {
      previewTrack = await captureTrack('video', settings.cameraId);
      await applyBlur(previewTrack);
      videoElement.srcObject = new MediaStream([previewTrack]);
      videoElement.play().catch(() => {});
      return { ok: true, blur: blurSupported(previewTrack) };
    } catch (error) {
      return { ok: false, blur: false, message: describeMediaError(error, 'à câmera') };
    }
  }

  function stopPreview() {
    if (previewTrack) {
      previewTrack.stop();
      previewTrack = null;
    }
  }

  async function startMicTest(onLevel) {
    stopMicTest();
    try {
      testTrack = await captureTrack('audio', settings.micId);
      testMeter = createMeter(testTrack);
      testTimer = setInterval(() => onLevel(Math.min(1, meterLevel(testMeter) * 6)), 80);
      return { ok: true };
    } catch (error) {
      return { ok: false, message: describeMediaError(error, 'ao microfone') };
    }
  }

  function stopMicTest() {
    clearInterval(testTimer);
    testTimer = null;
    closeMeter(testMeter);
    testMeter = null;
    if (testTrack) {
      testTrack.stop();
      testTrack = null;
    }
  }

  /* sons: entrada, saida e mesa de sons */

  function avatarColor(seed) {
    let hash = 0;
    for (const char of String(seed || '')) {
      hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
    }
    return AVATAR_COLORS[hash % AVATAR_COLORS.length];
  }

  function ensureMix() {
    if (!mix.destination) {
      const context = ensureAudioContext();
      mix.destination = context.createMediaStreamDestination();
      mix.soundBus = context.createGain();
      mix.soundBus.connect(mix.destination);
      mix.track = mix.destination.stream.getAudioTracks()[0] || null;
    }
    return mix;
  }

  function outgoingAudio() {
    try {
      return ensureMix().track || local.mic;
    } catch {
      return local.mic;
    }
  }

  function disconnectMic() {
    if (mix.micSource) {
      try {
        mix.micSource.disconnect();
      } catch {
        // o no ja estava desligado
      }
      mix.micSource = null;
    }
  }

  function connectMic() {
    disconnectMic();
    if (!local.mic) {
      return;
    }
    try {
      const context = ensureAudioContext();
      mix.micSource = context.createMediaStreamSource(new MediaStream([local.mic]));
      mix.micSource.connect(ensureMix().destination);
    } catch {
      mix.micSource = null;
    }
  }

  function playTone(kind) {
    if (local.deaf) {
      return;
    }
    try {
      const context = ensureAudioContext();
      let notes = [659.25, 440];
      if (kind === 'join') {
        notes = [587.33, 880];
      } else if (kind === 'live') {
        notes = [523.25, 783.99, 1046.5];
      }
      const start = context.currentTime + 0.01;
      notes.forEach((frequency, index) => {
        const oscillator = context.createOscillator();
        const gain = context.createGain();
        const at = start + index * 0.12;
        oscillator.type = 'sine';
        oscillator.frequency.setValueAtTime(frequency, at);
        gain.gain.setValueAtTime(0.0001, at);
        gain.gain.exponentialRampToValueAtTime(0.16, at + 0.02);
        gain.gain.exponentialRampToValueAtTime(0.0001, at + 0.28);
        oscillator.connect(gain);
        gain.connect(context.destination);
        oscillator.start(at);
        oscillator.stop(at + 0.3);
      });
    } catch {
      // sem saida de audio disponivel
    }
  }

  function playLiveTone() {
    playTone('live');
  }

  function loadSoundVolume() {
    try {
      const stored = localStorage.getItem(SOUND_VOLUME_KEY);
      const value = stored === null ? NaN : Number(stored);
      return Number.isFinite(value) && value >= 0 && value <= 1 ? value : 0.8;
    } catch {
      return 0.8;
    }
  }

  function setSoundVolume(value) {
    soundVolume = Math.max(0, Math.min(1, Number(value) || 0));
    try {
      localStorage.setItem(SOUND_VOLUME_KEY, String(soundVolume));
    } catch {
      // sem armazenamento local
    }
  }

  function getSoundVolume() {
    return soundVolume;
  }

  async function soundDuration(arrayBuffer) {
    const buffer = await ensureAudioContext().decodeAudioData(arrayBuffer.slice(0));
    return buffer.duration;
  }

  async function playSound(sound, loadData) {
    const context = ensureAudioContext();
    let buffer = soundBuffers.get(sound.id);
    if (!buffer) {
      const data = await loadData(sound.id);
      if (!data) {
        throw new Error('não encontrei esse som');
      }
      buffer = await context.decodeAudioData(data);
      soundBuffers.set(sound.id, buffer);
    }
    const source = context.createBufferSource();
    source.buffer = buffer;
    const gain = context.createGain();
    gain.gain.value = soundVolume;
    source.connect(gain);
    if (room) {
      gain.connect(ensureMix().soundBus);
    }
    if (!local.deaf) {
      gain.connect(context.destination);
    }
    source.onended = () => {
      try {
        gain.disconnect();
      } catch {
        // ja desligado
      }
    };
    source.start();
    source.stop(context.currentTime + MAX_SOUND_SECONDS);
    return Boolean(room);
  }

  function forgetSound(id) {
    soundBuffers.delete(id);
  }

  /* telhas */

  function localVideo() {
    if (!local.videoEl) {
      local.videoEl = document.createElement('video');
      local.videoEl.autoplay = true;
      local.videoEl.muted = true;
      local.videoEl.playsInline = true;
      local.videoEl.className = 'mirror';
    }
    const current = local.videoEl.srcObject ? local.videoEl.srcObject.getVideoTracks()[0] : null;
    if (local.camera && current !== local.camera) {
      local.videoEl.srcObject = new MediaStream([local.camera]);
    }
    return local.videoEl;
  }

  function tile(info, context) {
    const element = document.createElement('div');
    element.className = 'tile';
    element.dataset.voiceId = info.id;
    element.addEventListener('click', () => context.onFocus(info.id));
    element.addEventListener('contextmenu', (event) => {
      event.preventDefault();
      context.onContextMenu(info, event);
    });

    if (info.video) {
      element.classList.add('has-video');
      element.append(info.video);
      info.video.play().catch(() => {});
      const expand = document.createElement('button');
      expand.type = 'button';
      expand.className = 'tile-expand';
      expand.title = 'Tela cheia';
      expand.setAttribute('aria-label', 'Tela cheia');
      expand.innerHTML = EXPAND_ICON;
      expand.addEventListener('click', (event) => {
        event.stopPropagation();
        context.onFullscreen(info);
      });
      element.append(expand);
    } else if (info.thumb) {
      const preview = document.createElement('img');
      preview.className = 'tile-thumb';
      preview.src = info.thumb;
      preview.alt = '';
      element.append(preview);
    } else {
      const avatar = document.createElement('span');
      avatar.className = 'avatar tile-avatar';
      avatar.textContent = initial(info.name);
      avatar.style.background = avatarColor(info.name);
      const avatarUrl = context.avatarFor ? context.avatarFor(info) : null;
      if (avatarUrl) {
        avatar.textContent = '';
        avatar.classList.add('has-image');
        const image = document.createElement('img');
        image.src = avatarUrl;
        image.alt = '';
        avatar.append(image);
      }
      element.append(avatar);
    }

    const label = document.createElement('span');
    label.className = 'tile-label';
    if (info.deaf || !info.mic) {
      const icon = document.createElement('span');
      icon.className = 'tile-icon';
      icon.innerHTML = info.deaf ? ICONS.deaf : ICONS.micOff;
      label.append(icon);
    }
    label.append(document.createTextNode(info.self ? `${info.name} (você)` : info.name));
    element.append(label);

    if (!info.self && info.state !== 'connected') {
      const status = document.createElement('span');
      status.className = 'tile-status';
      status.textContent = info.state === 'failed' ? 'reconectando…' : 'conectando…';
      element.append(status);
    }

    if (info.live) {
      const badge = document.createElement('span');
      badge.className = 'live-badge';
      badge.textContent = 'AO VIVO';
      element.append(badge);
      const actions = document.createElement('div');
      actions.className = 'tile-actions';
      context.liveActions(info, actions);
      element.append(actions);
    }
    return element;
  }

  function streamTile(stream, context) {
    const element = document.createElement('div');
    element.className = 'tile stream-tile';
    element.dataset.tileId = stream.id;
    element.addEventListener('click', () => context.onFocus(stream.id));
    element.addEventListener('contextmenu', (event) => {
      event.preventDefault();
      context.onStreamMenu(stream, event);
    });

    const connected = stream.mode === 'embedded' && stream.video && stream.state === 'connected';
    if (connected) {
      element.classList.add('has-video');
      element.append(stream.video);
      stream.video.play().catch(() => {});
    } else {
      const note = document.createElement('div');
      note.className = 'stream-note';
      const text = document.createElement('span');
      text.textContent = stream.mode === 'native'
        ? `A tela de ${stream.name} está aberta em outra janela`
        : `Conectando à tela de ${stream.name}…`;
      note.append(text);
      if (stream.mode === 'native') {
        const back = document.createElement('button');
        back.type = 'button';
        back.className = 'button small';
        back.textContent = 'Trazer para o app';
        back.addEventListener('click', (event) => {
          event.stopPropagation();
          context.onPopin(stream);
        });
        note.append(back);
      }
      element.append(note);
    }

    const label = document.createElement('span');
    label.className = 'tile-label';
    label.textContent = `Tela de ${stream.name}`;
    const badge = document.createElement('span');
    badge.className = 'live-badge';
    badge.textContent = 'AO VIVO';
    element.append(label, badge);

    const tools = document.createElement('div');
    tools.className = 'stream-tools';
    const tool = (title, icon, handler) => {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'stream-tool';
      button.title = title;
      button.setAttribute('aria-label', title);
      button.innerHTML = icon;
      button.addEventListener('click', (event) => {
        event.stopPropagation();
        handler(stream);
      });
      return button;
    };
    if (connected) {
      tools.append(tool('Tela cheia', EXPAND_ICON, context.onStreamFullscreen));
    }
    if (stream.mode === 'embedded') {
      tools.append(tool('Abrir em outra janela', POPOUT_ICON, context.onPopout));
    }
    tools.append(tool('Parar de assistir', CLOSE_ICON, context.onStopWatching));
    element.append(tools);
    return element;
  }

  function renderTiles(container, context) {
    if (!room) {
      container.replaceChildren();
      return;
    }
    const streamTiles = (context.streams || []).map((stream) => streamTile(stream, context));
    const tiles = [tile({
      id: selfId,
      self: true,
      memberId: context.selfMemberId,
      name: context.selfName,
      mic: micLive(),
      deaf: local.deaf,
      video: local.cameraOn && local.camera ? localVideo() : (context.selfPreview || null),
      live: context.outgoingLive ? { name: context.outgoingName } : null,
      state: 'connected',
    }, context)];

    for (const participant of participants()) {
      const peer = peers.get(participant.id);
      const showVideo = Boolean(participant.voice.camera && peer && peer.videoEl && peer.videoTrack
        && !peer.videoTrack.muted && !personPrefs(prefsKey(participant)).videoHidden);
      tiles.push(tile({
        id: participant.id,
        self: false,
        memberId: participant.memberId,
        name: context.nameFor(participant),
        mic: participant.voice.mic,
        deaf: participant.voice.deaf,
        video: showVideo ? peer.videoEl : null,
        thumb: !showVideo && participant.voice.live && context.thumbFor
          ? context.thumbFor(participant.id)
          : null,
        live: participant.voice.live ? { name: participant.voice.liveName } : null,
        state: peer ? peer.state : 'new',
      }, context));
    }
    tiles.unshift(...streamTiles);
    const tileId = (item) => item.dataset.tileId || item.dataset.voiceId;
    const focused = context.focusId ? tiles.find((item) => tileId(item) === context.focusId) : null;
    container.classList.toggle('focused', Boolean(focused));
    container.dataset.count = String(Math.min(tiles.length, 9));
    if (!focused) {
      container.replaceChildren(...tiles);
      return;
    }
    focused.classList.add('focus');
    const strip = document.createElement('div');
    strip.className = 'tile-strip';
    strip.append(...tiles.filter((item) => item !== focused));
    container.replaceChildren(focused, strip);
  }

  function init(id) {
    selfId = id;
  }

  function state() {
    return {
      room: currentRoom(),
      busy,
      micOn: local.micOn,
      micLive: micLive(),
      deaf: local.deaf,
      cameraOn: local.cameraOn,
      hasMic: Boolean(local.mic),
      micError: local.micError,
      cameraError: local.cameraError,
      settings: { ...settings },
      blur: blurSupported(),
    };
  }

  return {
    blurSupported,
    currentRoom,
    handleRtc,
    inRoom,
    init,
    join,
    leave,
    listDevices,
    onStats,
    participants,
    renderTiles,
    roomPeople,
    setBlur,
    setCameraOn,
    setDeaf,
    setDevice,
    setMicOn,
    setPresence,
    startMicTest,
    startPreview,
    state,
    stopMicTest,
    stopPreview,
    subscribe,
    syncVoice,
    personPrefs,
    prefsKey,
    setPersonPrefs,
    videoStream,
    avatarColor,
    forgetSound,
    getSoundVolume,
    playLiveTone,
    playSound,
    setSoundVolume,
    soundDuration,
  };
})();
