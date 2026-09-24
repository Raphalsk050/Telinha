'use strict';

const crypto = require('node:crypto');
const { EventEmitter } = require('node:events');

const ID_PATTERN = /^[A-Za-z0-9_-]{8,64}$/;
const MAX_SDP_LENGTH = 20000;
const WANT_TIMEOUT_MS = 10000;
const CONNECT_TIMEOUT_MS = 30000;
const STALL_TIMEOUT_MS = 30000;
// Quem entrega le do disco so quando a fila do canal esvazia, e numa conexao lenta isso demora.
const SERVE_STALL_TIMEOUT_MS = 120000;
const OFFER_TIMEOUT_MS = 30000;
const FINISH_WAIT_MS = 2000;
const RETRY_DELAY_MS = 3000;
const MAX_ATTEMPTS = 3;
const MAX_DOWNLOADS = 3;
const MAX_SERVES = 6;

// Arquivos grandes vao direto de um computador para o outro. O servidor de mensagens so carrega
// a conversa curta: quem quer o arquivo pergunta quem tem (file-want), quem tem responde
// (file-have), e os dois trocam as descricoes da conexao WebRTC (file-signal). Os bytes passam
// pelo canal de dados que a janela abre, e aqui ficam a leitura, a escrita e a conferencia.
class FileShare extends EventEmitter {
  constructor({
    publish, selfId, findLocal, openReader, createWriter,
  }) {
    super();
    this.publish = publish;
    this.selfId = selfId;
    this.findLocal = findLocal;
    this.openReader = openReader;
    this.createWriter = createWriter;
    this.downloads = new Map();
    this.serves = new Map();
  }

  /* recebendo */

  findDownload(key, fileId) {
    return [...this.downloads.values()].find((item) => item.target.key === key && item.target.fileId === fileId);
  }

  download(target) {
    if (this.findDownload(target.key, target.fileId)) {
      return;
    }
    const download = {
      id: crypto.randomUUID(),
      target,
      state: 'queued',
      attempts: 0,
      requestId: null,
      holder: null,
      writer: null,
      bytes: 0,
      timer: null,
    };
    this.downloads.set(download.id, download);
    this.status(download, 'waiting');
    this.pump();
  }

  pump() {
    let active = [...this.downloads.values()].filter((item) => item.state !== 'queued').length;
    for (const download of this.downloads.values()) {
      if (active >= MAX_DOWNLOADS) {
        return;
      }
      if (download.state === 'queued') {
        active += 1;
        this.search(download);
      }
    }
  }

  search(download) {
    if (this.downloads.get(download.id) !== download) {
      return;
    }
    download.attempts += 1;
    download.state = 'searching';
    download.requestId = crypto.randomUUID();
    download.holder = null;
    download.bytes = 0;
    const sent = this.publish(download.target.spaceId, 'file-want', {
      offerId: download.target.offerId, requestId: download.requestId,
    });
    if (!sent) {
      this.fail(download, 'offline');
      return;
    }
    this.status(download, 'searching');
    this.arm(download, WANT_TIMEOUT_MS, 'nobody');
  }

  arm(download, delay, reason) {
    clearTimeout(download.timer);
    download.timer = setTimeout(() => this.fail(download, reason), delay);
  }

  handleHave(spaceId, message) {
    if (message.to !== this.selfId || typeof message.from !== 'string') {
      return;
    }
    const download = [...this.downloads.values()].find((item) => item.requestId === message.requestId
      && item.target.spaceId === spaceId);
    if (!download || download.state !== 'searching') {
      return;
    }
    download.state = 'connecting';
    download.holder = message.from;
    this.arm(download, CONNECT_TIMEOUT_MS, 'unreachable');
    this.emit('start', {
      id: download.id,
      role: 'receive',
      key: download.target.key,
      fileId: download.target.fileId,
      size: download.target.size,
    });
  }

  chunk(id, data) {
    const download = this.downloads.get(id);
    if (!download || (download.state !== 'connecting' && download.state !== 'receiving')) {
      return;
    }
    if (!download.writer) {
      try {
        download.writer = this.createWriter(download.target);
      } catch {
        this.fail(download, 'disk');
        return;
      }
    }
    if (download.state === 'connecting') {
      download.state = 'receiving';
      this.status(download, 'receiving');
    }
    download.bytes += data.length;
    if (download.bytes > download.target.size) {
      this.fail(download, 'corrupt');
      return;
    }
    download.writer.write(data).catch(() => {});
    this.arm(download, STALL_TIMEOUT_MS, 'stalled');
  }

  async finish(id) {
    const download = this.downloads.get(id);
    if (!download || download.state !== 'receiving') {
      return false;
    }
    for (let waited = 0; download.bytes < download.target.size && waited < FINISH_WAIT_MS; waited += 50) {
      await new Promise((resolve) => {
        setTimeout(resolve, 50);
      });
    }
    if (this.downloads.get(id) !== download || download.state !== 'receiving') {
      return false;
    }
    if (download.bytes !== download.target.size) {
      this.fail(download, 'corrupt');
      return false;
    }
    download.state = 'finishing';
    clearTimeout(download.timer);
    const { writer } = download;
    download.writer = null;
    let result = null;
    try {
      result = await writer.finish({ size: download.target.size, sha256: download.target.sha256 });
    } catch {
      result = null;
    }
    if (!result) {
      this.fail(download, 'corrupt');
      return false;
    }
    this.downloads.delete(id);
    this.emit('complete', { target: download.target, secret: { key: result.key, iv: result.iv } });
    this.pump();
    return true;
  }

  fail(download, reason) {
    if (this.downloads.get(download.id) !== download) {
      return;
    }
    clearTimeout(download.timer);
    if (download.writer) {
      download.writer.abort();
      download.writer = null;
    }
    if (download.state === 'connecting' || download.state === 'receiving') {
      this.emit('stop', { id: download.id });
    }
    download.holder = null;
    download.requestId = null;
    if (download.attempts < MAX_ATTEMPTS && reason !== 'offline') {
      download.state = 'retrying';
      this.status(download, 'searching');
      download.timer = setTimeout(() => this.search(download), RETRY_DELAY_MS);
      return;
    }
    this.downloads.delete(download.id);
    this.status(download, 'failed', reason);
    this.pump();
  }

  status(download, status, reason = null) {
    const {
      spaceId, channelId, key, fileId,
    } = download.target;
    this.emit('status', {
      spaceId, channelId, key, fileId, status, reason,
    });
  }

  /* entregando */

  handleWant(spaceId, message) {
    const { offerId, requestId, from } = message;
    if (typeof from !== 'string' || !ID_PATTERN.test(String(offerId)) || !ID_PATTERN.test(String(requestId))) {
      return;
    }
    const serves = [...this.serves.values()];
    if (serves.length >= MAX_SERVES || serves.some((item) => item.peer === from && item.requestId === requestId)) {
      return;
    }
    const local = this.findLocal(spaceId, offerId);
    if (!local) {
      return;
    }
    const serve = {
      id: crypto.randomUUID(),
      spaceId,
      peer: from,
      requestId,
      local,
      state: 'offered',
      reader: null,
      iterator: null,
      reading: Promise.resolve(),
      timer: null,
    };
    this.serves.set(serve.id, serve);
    this.armServe(serve, OFFER_TIMEOUT_MS);
    this.publish(spaceId, 'file-have', { to: from, requestId });
  }

  armServe(serve, delay) {
    clearTimeout(serve.timer);
    serve.timer = setTimeout(() => this.endServe(serve), delay);
  }

  read(id) {
    const serve = this.serves.get(id);
    if (!serve) {
      return Promise.resolve(null);
    }
    const next = serve.reading.then(async () => {
      if (this.serves.get(id) !== serve) {
        return null;
      }
      if (!serve.iterator) {
        serve.reader = this.openReader(serve.local);
        serve.iterator = serve.reader[Symbol.asyncIterator]();
        serve.state = 'sending';
      }
      const { value, done } = await serve.iterator.next();
      this.armServe(serve, SERVE_STALL_TIMEOUT_MS);
      return done ? null : value;
    });
    serve.reading = next.catch(() => {});
    return next;
  }

  endServe(serve) {
    if (this.serves.get(serve.id) !== serve) {
      return;
    }
    this.serves.delete(serve.id);
    clearTimeout(serve.timer);
    if (serve.reader) {
      serve.reader.destroy();
    }
    this.emit('stop', { id: serve.id });
  }

  /* conversa entre as duas pontas */

  handleSignal(spaceId, message) {
    if (message.to !== this.selfId || typeof message.from !== 'string' || typeof message.sdp !== 'string'
      || message.sdp.length > MAX_SDP_LENGTH) {
      return;
    }
    const download = [...this.downloads.values()].find((item) => item.requestId === message.requestId
      && item.holder === message.from && item.target.spaceId === spaceId);
    if (download) {
      this.emit('signal', { id: download.id, sdp: message.sdp });
      return;
    }
    const serve = [...this.serves.values()].find((item) => item.requestId === message.requestId
      && item.peer === message.from && item.spaceId === spaceId);
    if (!serve || serve.state !== 'offered') {
      return;
    }
    serve.state = 'connecting';
    this.armServe(serve, CONNECT_TIMEOUT_MS);
    this.emit('start', {
      id: serve.id,
      role: 'serve',
      key: serve.local.key,
      fileId: serve.local.fileId,
      size: serve.local.size,
      sdp: message.sdp,
    });
  }

  signal(id, sdp) {
    if (typeof sdp !== 'string' || sdp.length > MAX_SDP_LENGTH) {
      return false;
    }
    const download = this.downloads.get(id);
    if (download && download.holder) {
      return this.publish(download.target.spaceId, 'file-signal', {
        to: download.holder, requestId: download.requestId, sdp,
      });
    }
    const serve = this.serves.get(id);
    if (serve) {
      return this.publish(serve.spaceId, 'file-signal', { to: serve.peer, requestId: serve.requestId, sdp });
    }
    return false;
  }

  close(id, reason) {
    const download = this.downloads.get(id);
    if (download) {
      if (download.state === 'connecting' || download.state === 'receiving') {
        this.fail(download, reason ? String(reason).slice(0, 40) : 'closed');
      }
      return;
    }
    const serve = this.serves.get(id);
    if (serve) {
      this.endServe(serve);
    }
  }

  stopAll() {
    for (const download of this.downloads.values()) {
      clearTimeout(download.timer);
      if (download.writer) {
        download.writer.abort();
      }
    }
    this.downloads.clear();
    for (const serve of [...this.serves.values()]) {
      this.endServe(serve);
    }
  }
}

module.exports = { FileShare };
