'use strict';

const StreamView = (() => {
  const api = window.telinha;
  const ICE_SERVERS = [{ urls: ['stun:stun.l.google.com:19302', 'stun:stun.cloudflare.com:3478'] }];
  const GATHER_TIMEOUT_MS = 3000;
  const STATS_INTERVAL_MS = 1000;

  const SELF = 'self';
  const views = new Map();
  const listeners = new Set();
  const statsListeners = new Set();
  let volumeFor = () => 100;
  let deafFor = () => false;
  let sinkId = '';
  let audioContext = null;

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

  function configure(options) {
    volumeFor = options.volumeFor;
    deafFor = options.deafFor;
    sinkId = options.sinkId || '';
  }

  function hiddenAudio(muted) {
    const element = document.createElement('audio');
    element.autoplay = true;
    element.hidden = true;
    element.muted = muted;
    document.body.append(element);
    return element;
  }

  function closeBoost(view) {
    if (view.boost) {
      try {
        view.boost.source.disconnect();
        view.boost.gain.disconnect();
      } catch {
        // os nos ja estavam desligados
      }
      view.boost = null;
    }
  }

  function applyContextSink() {
    if (audioContext && typeof audioContext.setSinkId === 'function') {
      audioContext.setSinkId(sinkId).catch(() => {});
    }
  }

  function ensureAudioContext() {
    if (!audioContext) {
      audioContext = new AudioContext();
      applyContextSink();
    }
    if (audioContext.state === 'suspended') {
      audioContext.resume().catch(() => {});
    }
    return audioContext;
  }

  // Acima de 100% o som sai direto do AudioContext, sem passar por outra MediaStream e outro
  // elemento de audio, que so acrescentavam uma troca de relogio no caminho.
  function applyAudio(view) {
    if (!view.audioStream || view.mine) {
      return;
    }
    const volume = Math.max(0, Math.min(200, Number(volumeFor(view.sharerId)) || 0));
    if (volume > 100 && !view.boost) {
      try {
        const context = ensureAudioContext();
        const source = context.createMediaStreamSource(view.audioStream);
        const gain = context.createGain();
        source.connect(gain);
        gain.connect(context.destination);
        view.boost = { source, gain };
      } catch {
        view.boost = null;
      }
    } else if (volume <= 100) {
      closeBoost(view);
    }
    if (view.audio.srcObject !== view.audioStream) {
      view.audio.srcObject = view.audioStream;
    }
    if (view.boost) {
      view.boost.gain.gain.value = deafFor() ? 0 : volume / 100;
      view.audio.volume = 1;
      view.audio.muted = true;
    } else {
      view.audio.volume = Math.min(1, volume / 100);
      view.audio.muted = deafFor();
    }
    if (sinkId && typeof view.audio.setSinkId === 'function') {
      view.audio.setSinkId(sinkId).catch(() => {});
    }
    view.audio.play().catch(() => {});
  }

  function refreshAudio() {
    for (const view of views.values()) {
      applyAudio(view);
    }
  }

  function setSink(id) {
    sinkId = String(id || '');
    applyContextSink();
    refreshAudio();
  }

  function waitForGathering(pc) {
    if (pc.iceGatheringState === 'complete') {
      return Promise.resolve();
    }
    return new Promise((resolve) => {
      let timer = null;
      const done = () => {
        clearTimeout(timer);
        pc.removeEventListener('icegatheringstatechange', check);
        resolve();
      };
      function check() {
        if (pc.iceGatheringState === 'complete') {
          done();
        }
      }
      timer = setTimeout(done, GATHER_TIMEOUT_MS);
      pc.addEventListener('icegatheringstatechange', check);
    });
  }

  async function collectStats(view) {
    if (!view.pc || views.get(view.sharerId) !== view) {
      return;
    }
    try {
      const report = await view.pc.getStats();
      let pair = null;
      let video = null;
      report.forEach((stat) => {
        if (stat.type === 'transport' && stat.selectedCandidatePairId) {
          pair = report.get(stat.selectedCandidatePairId) || pair;
        } else if (stat.type === 'inbound-rtp' && stat.kind === 'video') {
          video = stat;
        }
      });
      const now = performance.now();
      const lost = video ? video.packetsLost || 0 : 0;
      const received = video ? video.packetsReceived || 0 : 0;
      const bytes = video ? video.bytesReceived || 0 : 0;
      const previous = view.last;
      const deltaLost = previous ? Math.max(0, lost - previous.lost) : 0;
      const deltaReceived = previous ? Math.max(0, received - previous.received) : 0;
      const seconds = previous ? (now - previous.at) / 1000 : 0;
      view.last = { at: now, lost, received, bytes };
      view.stats = {
        rtt: pair && Number.isFinite(pair.currentRoundTripTime) ? pair.currentRoundTripTime * 1000 : null,
        loss: deltaLost + deltaReceived > 0 ? deltaLost / (deltaLost + deltaReceived) : 0,
        fps: video && Number.isFinite(video.framesPerSecond) ? video.framesPerSecond : null,
        width: video ? video.frameWidth : null,
        height: video ? video.frameHeight : null,
        bitrate: previous && seconds > 0 ? ((bytes - previous.bytes) * 8) / seconds : null,
        jitter: video && video.jitterBufferEmittedCount
          ? (video.jitterBufferDelay / video.jitterBufferEmittedCount) * 1000
          : null,
        dropped: video ? video.framesDropped : null,
        decoder: video ? video.decoderImplementation : null,
      };
      for (const listener of statsListeners) {
        listener(view.sharerId, view.stats);
      }
    } catch {
      // a conexao fechou no meio da leitura
    }
  }

  function close(sharerId) {
    const view = views.get(sharerId);
    if (!view) {
      return;
    }
    views.delete(sharerId);
    clearInterval(view.statsTimer);
    if (view.pc) {
      view.pc.close();
    }
    closeBoost(view);
    view.video.srcObject = null;
    view.audio.srcObject = null;
    view.audio.remove();
    view.keepAlive.srcObject = null;
    view.keepAlive.remove();
    emit();
  }

  async function handleOffer({ sharerId, sdp }) {
    close(sharerId);
    const video = document.createElement('video');
    video.autoplay = true;
    video.playsInline = true;
    video.muted = true;
    video.className = 'stream-video';
    const view = {
      sharerId,
      pc: new RTCPeerConnection({ iceServers: ICE_SERVERS, bundlePolicy: 'max-bundle' }),
      video,
      mine: sharerId === SELF,
      audio: hiddenAudio(sharerId === SELF),
      keepAlive: hiddenAudio(true),
      videoTrack: null,
      audioStream: null,
      boost: null,
      state: 'connecting',
      stats: null,
      last: null,
      statsTimer: null,
    };
    views.set(sharerId, view);
    const { pc } = view;

    pc.ontrack = ({ track, receiver }) => {
      try {
        receiver.jitterBufferTarget = 0;
      } catch {
        // versao sem ajuste do buffer
      }
      if (track.kind === 'video') {
        view.videoTrack = track;
        view.video.srcObject = new MediaStream([track]);
        view.video.play().catch(() => {});
      } else {
        view.audioStream = new MediaStream([track]);
        view.keepAlive.srcObject = view.audioStream;
        view.keepAlive.play().catch(() => {});
        applyAudio(view);
      }
      emit();
    };
    pc.onconnectionstatechange = () => {
      if (views.get(sharerId) !== view) {
        return;
      }
      view.state = pc.connectionState;
      if (pc.connectionState === 'connected') {
        api.streamEmbeddedState(sharerId, 'live').catch(() => {});
        clearInterval(view.statsTimer);
        view.statsTimer = setInterval(() => collectStats(view), STATS_INTERVAL_MS);
      } else if (pc.connectionState === 'failed') {
        api.streamEmbeddedState(sharerId, 'failed').catch(() => {});
      }
      emit();
    };

    try {
      await pc.setRemoteDescription({ type: 'offer', sdp });
      await pc.setLocalDescription(await pc.createAnswer());
      await waitForGathering(pc);
      if (views.get(sharerId) === view) {
        await api.streamAnswer(sharerId, pc.localDescription.sdp);
      }
    } catch {
      if (views.get(sharerId) === view) {
        view.state = 'failed';
        api.streamEmbeddedState(sharerId, 'failed').catch(() => {});
      }
    }
    emit();
  }

  function sync(incoming, keepSelf) {
    const embedded = new Set(incoming.filter((entry) => entry.mode === 'embedded').map((entry) => entry.sharerId));
    if (keepSelf) {
      embedded.add(SELF);
    }
    for (const sharerId of [...views.keys()]) {
      if (!embedded.has(sharerId)) {
        close(sharerId);
      }
    }
  }

  function video(sharerId) {
    const view = views.get(sharerId);
    return view && view.videoTrack ? view.video : null;
  }

  function mediaStream(sharerId) {
    const view = views.get(sharerId);
    return view && view.videoTrack ? new MediaStream([view.videoTrack]) : null;
  }

  function state(sharerId) {
    const view = views.get(sharerId);
    return view ? view.state : null;
  }

  return {
    SELF,
    close,
    configure,
    handleOffer,
    mediaStream,
    onStats,
    refreshAudio,
    setSink,
    state,
    subscribe,
    sync,
    video,
  };
})();
