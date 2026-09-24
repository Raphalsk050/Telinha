'use strict';

// Os arquivos grandes do chat vao por um canal de dados WebRTC entre os dois computadores. O
// processo principal decide quem entrega para quem e guarda os bytes; aqui ficam a conexao, o
// envio com controle de fila e o progresso que aparece no cartao do arquivo.
const FileTransfer = (() => {
  const api = window.telinha;
  const ICE_SERVERS = [{ urls: ['stun:stun.l.google.com:19302', 'stun:stun.cloudflare.com:3478'] }];
  const GATHER_TIMEOUT_MS = 3000;
  const MESSAGE_BYTES = 64 * 1024;
  const HIGH_WATER_BYTES = 4 * 1024 * 1024;
  const LOW_WATER_BYTES = 1024 * 1024;
  const LINGER_MS = 30000;
  const PROGRESS_INTERVAL_MS = 200;

  const sessions = new Map();
  const listeners = new Set();
  const reportedAt = new Map();

  function onProgress(listener) {
    listeners.add(listener);
    return () => listeners.delete(listener);
  }

  // O mesmo arquivo pode estar indo para varias pessoas ao mesmo tempo, entao o progresso sai somado.
  function report(session, force = false) {
    const fileKey = `${session.role}/${session.key}/${session.fileId}`;
    const now = performance.now();
    if (!force && now - (reportedAt.get(fileKey) ?? 0) < PROGRESS_INTERVAL_MS) {
      return;
    }
    reportedAt.set(fileKey, now);
    const related = [...sessions.values()].filter((item) => item.role === session.role
      && item.key === session.key && item.fileId === session.fileId);
    const info = {
      key: session.key,
      fileId: session.fileId,
      direction: session.role === 'serve' ? 'up' : 'down',
      done: related.reduce((sum, item) => sum + item.done, 0),
      total: related.reduce((sum, item) => sum + item.size, 0),
      peers: related.length,
    };
    for (const listener of listeners) {
      listener(info);
    }
  }

  function waitForGathering(pc) {
    if (pc.iceGatheringState === 'complete') {
      return Promise.resolve();
    }
    return new Promise((resolve) => {
      let timer = null;
      const check = () => {
        if (pc.iceGatheringState === 'complete') {
          done();
        }
      };
      const done = () => {
        clearTimeout(timer);
        pc.removeEventListener('icegatheringstatechange', check);
        resolve();
      };
      timer = setTimeout(done, GATHER_TIMEOUT_MS);
      pc.addEventListener('icegatheringstatechange', check);
    });
  }

  function drained(channel) {
    return new Promise((resolve) => {
      const done = () => {
        channel.removeEventListener('bufferedamountlow', done);
        channel.removeEventListener('close', done);
        resolve();
      };
      channel.addEventListener('bufferedamountlow', done);
      channel.addEventListener('close', done);
    });
  }

  function close(session, reason = null) {
    if (sessions.get(session.id) !== session) {
      return;
    }
    sessions.delete(session.id);
    clearTimeout(session.linger);
    if (session.pc) {
      session.pc.close();
      session.pc = null;
    }
    if (!session.silent) {
      api.fileClose(session.id, reason).catch(() => {});
    }
    report(session, true);
  }

  /* recebendo */

  async function finishReceive(session) {
    session.finished = true;
    await api.fileFinish(session.id).catch(() => false);
    session.silent = true;
    close(session);
  }

  async function receive(session) {
    const pc = new RTCPeerConnection({ iceServers: ICE_SERVERS });
    session.pc = pc;
    const channel = pc.createDataChannel('file', { ordered: true });
    channel.binaryType = 'arraybuffer';
    channel.onmessage = ({ data }) => {
      if (session.finished || !(data instanceof ArrayBuffer)) {
        return;
      }
      session.done += data.byteLength;
      api.fileChunk(session.id, data);
      report(session);
      if (session.done >= session.size) {
        finishReceive(session);
      }
    };
    channel.onclose = () => {
      if (!session.finished) {
        close(session, 'closed');
      }
    };
    pc.onconnectionstatechange = () => {
      if (pc.connectionState === 'failed' && !session.finished) {
        close(session, 'failed');
      }
    };
    await pc.setLocalDescription(await pc.createOffer());
    await waitForGathering(pc);
    if (sessions.get(session.id) === session) {
      await api.fileSignal(session.id, pc.localDescription.sdp);
    }
  }

  /* entregando */

  async function pump(session, channel) {
    channel.bufferedAmountLowThreshold = LOW_WATER_BYTES;
    let pending = api.fileRead(session.id);
    for (;;) {
      const chunk = await pending;
      if (!chunk || sessions.get(session.id) !== session) {
        break;
      }
      pending = api.fileRead(session.id);
      for (let offset = 0; offset < chunk.byteLength; offset += MESSAGE_BYTES) {
        if (channel.bufferedAmount > HIGH_WATER_BYTES) {
          await drained(channel);
        }
        if (channel.readyState !== 'open') {
          return;
        }
        const piece = chunk.subarray(offset, offset + MESSAGE_BYTES);
        channel.send(piece);
        session.done += piece.byteLength;
      }
      report(session);
    }
    session.finished = true;
    report(session, true);
    // Quem recebe fecha a conexao quando junta o ultimo byte. Se isso nao vier, a sessao some sozinha.
    session.linger = setTimeout(() => close(session), LINGER_MS);
  }

  async function serve(session, offer) {
    const pc = new RTCPeerConnection({ iceServers: ICE_SERVERS });
    session.pc = pc;
    pc.ondatachannel = ({ channel }) => {
      channel.binaryType = 'arraybuffer';
      channel.onclose = () => close(session, session.finished ? null : 'closed');
      const begin = () => pump(session, channel).catch(() => close(session, 'failed'));
      if (channel.readyState === 'open') {
        begin();
      } else {
        channel.onopen = begin;
      }
    };
    pc.onconnectionstatechange = () => {
      if (pc.connectionState === 'failed') {
        close(session, 'failed');
      }
    };
    await pc.setRemoteDescription({ type: 'offer', sdp: offer });
    await pc.setLocalDescription(await pc.createAnswer());
    await waitForGathering(pc);
    if (sessions.get(session.id) === session) {
      await api.fileSignal(session.id, pc.localDescription.sdp);
    }
  }

  /* eventos do processo principal */

  function start(payload) {
    if (!payload || sessions.has(payload.id)) {
      return;
    }
    const session = {
      id: payload.id,
      role: payload.role === 'serve' ? 'serve' : 'receive',
      key: payload.key,
      fileId: payload.fileId,
      size: Number(payload.size) || 0,
      done: 0,
      pc: null,
      finished: false,
      silent: false,
      linger: null,
    };
    sessions.set(session.id, session);
    const run = session.role === 'serve' ? serve(session, payload.sdp) : receive(session);
    run.catch(() => close(session, 'failed'));
  }

  function answer({ id, sdp }) {
    const session = sessions.get(id);
    if (!session || session.role !== 'receive' || !session.pc) {
      return;
    }
    session.pc.setRemoteDescription({ type: 'answer', sdp }).catch(() => close(session, 'failed'));
  }

  function stop({ id }) {
    const session = sessions.get(id);
    if (session) {
      session.silent = true;
      close(session);
    }
  }

  function bind() {
    api.onFileStart(start);
    api.onFileSignal(answer);
    api.onFileStop(stop);
  }

  return { bind, onProgress };
})();
