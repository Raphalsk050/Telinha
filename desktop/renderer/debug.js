'use strict';

const Debug = (() => {
  const MAX_SAMPLES = 90;
  const MAX_LOG_LINES = 600;
  const find = (id) => document.getElementById(id);

  const nodes = {
    dialog: find('debug-dialog'),
    close: find('debug-close'),
    tabs: find('debug-tabs'),
    summary: find('debug-summary'),
    ping: find('debug-ping'),
    pingValue: find('debug-ping-value'),
    loss: find('debug-loss'),
    lossValue: find('debug-loss-value'),
    stats: find('debug-stats'),
    copy: find('debug-copy'),
    log: find('log'),
  };

  const sources = new Map();
  let selected = null;
  let logLines = [];

  function update(key, { label, rtt = null, loss = null, rows = [], summary = '' }) {
    let source = sources.get(key);
    if (!source) {
      source = { label, samples: [], rows: [], summary: '' };
      sources.set(key, source);
    }
    source.label = label;
    source.rows = rows;
    source.summary = summary;
    source.samples.push({
      at: Date.now(),
      rtt: Number.isFinite(rtt) ? rtt : null,
      loss: Number.isFinite(loss) ? loss * 100 : null,
    });
    if (source.samples.length > MAX_SAMPLES) {
      source.samples.splice(0, source.samples.length - MAX_SAMPLES);
    }
    if (!selected || !sources.has(selected)) {
      selected = key;
    }
    render();
  }

  function remove(key) {
    sources.delete(key);
    if (selected === key) {
      selected = sources.keys().next().value ?? null;
    }
    render();
  }

  function appendLog(line) {
    logLines.push(line);
    if (logLines.length > MAX_LOG_LINES) {
      logLines.splice(0, logLines.length - MAX_LOG_LINES);
    }
    if (isOpen()) {
      const stick = nodes.log.scrollHeight - nodes.log.scrollTop - nodes.log.clientHeight < 40;
      nodes.log.textContent = logLines.join('\n');
      if (stick) {
        nodes.log.scrollTop = nodes.log.scrollHeight;
      }
    }
  }

  function quality(key) {
    const source = sources.get(key);
    const last = source ? source.samples[source.samples.length - 1] : null;
    if (!last || last.rtt === null) {
      return 'unknown';
    }
    const loss = last.loss ?? 0;
    if (last.rtt < 80 && loss < 2) {
      return 'good';
    }
    if (last.rtt < 200 && loss < 8) {
      return 'medium';
    }
    return 'bad';
  }

  function average(samples, key) {
    const values = samples.map((sample) => sample[key]).filter((value) => value !== null);
    return values.length === 0 ? null : values.reduce((sum, value) => sum + value, 0) / values.length;
  }

  function drawGraph(canvas, samples, key, minimumScale, step, color) {
    const ratio = window.devicePixelRatio || 1;
    const width = canvas.clientWidth || canvas.width;
    const height = canvas.clientHeight || canvas.height;
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
    const context = canvas.getContext('2d');
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.clearRect(0, 0, width, height);

    const values = samples.map((sample) => sample[key]);
    const peak = Math.max(minimumScale, ...values.filter((value) => value !== null));
    const scale = Math.ceil(peak / step) * step;
    const styles = getComputedStyle(document.documentElement);

    context.strokeStyle = styles.getPropertyValue('--line-soft').trim();
    context.fillStyle = styles.getPropertyValue('--muted').trim();
    context.font = '11px Segoe UI, sans-serif';
    context.lineWidth = 1;
    for (let line = 0; line <= 2; line += 1) {
      const y = 8 + ((height - 16) * line) / 2;
      context.beginPath();
      context.moveTo(0, y);
      context.lineTo(width, y);
      context.stroke();
      context.fillText(`${Math.round(scale - (scale * line) / 2)}`, 4, Math.max(18, y - 2));
    }

    context.strokeStyle = styles.getPropertyValue(color).trim();
    context.lineWidth = 2;
    context.beginPath();
    let started = false;
    values.forEach((value, index) => {
      if (value === null) {
        started = false;
        return;
      }
      const x = width - ((values.length - 1 - index) * width) / (MAX_SAMPLES - 1);
      const y = 8 + (height - 16) * (1 - Math.min(value, scale) / scale);
      if (started) {
        context.lineTo(x, y);
      } else {
        context.moveTo(x, y);
        started = true;
      }
    });
    context.stroke();
  }

  function renderTabs() {
    nodes.tabs.replaceChildren(...[...sources].map(([key, source]) => {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'tab';
      button.setAttribute('aria-selected', String(key === selected));
      button.textContent = source.label;
      button.addEventListener('click', () => {
        selected = key;
        render();
      });
      return button;
    }));
  }

  function render() {
    if (!isOpen()) {
      return;
    }
    renderTabs();
    const source = selected ? sources.get(selected) : null;
    const samples = source ? source.samples : [];
    nodes.summary.textContent = source
      ? source.summary
      : 'Nada conectado agora. Entre numa chamada ou transmissão para ver os números.';

    const last = samples[samples.length - 1];
    const averagePing = average(samples, 'rtt');
    nodes.pingValue.textContent = last && last.rtt !== null
      ? `${Math.round(last.rtt)} ms · média ${Math.round(averagePing)} ms`
      : '—';
    nodes.lossValue.textContent = last && last.loss !== null ? `${last.loss.toFixed(2)} %` : '—';
    drawGraph(nodes.ping, samples, 'rtt', 50, 25, '--accent');
    drawGraph(nodes.loss, samples, 'loss', 2, 1, '--danger');

    nodes.stats.replaceChildren(...(source ? source.rows : []).map(([label, value]) => {
      const wrapper = document.createElement('div');
      const term = document.createElement('dt');
      term.textContent = label;
      const detail = document.createElement('dd');
      detail.textContent = value;
      wrapper.append(term, detail);
      return wrapper;
    }));
    nodes.log.textContent = logLines.join('\n');
  }

  function isOpen() {
    return !nodes.dialog.hidden;
  }

  function open(key = null) {
    if (key && sources.has(key)) {
      selected = key;
    }
    nodes.dialog.hidden = false;
    render();
    nodes.log.scrollTop = nodes.log.scrollHeight;
  }

  function close() {
    nodes.dialog.hidden = true;
  }

  nodes.close.addEventListener('click', close);
  nodes.dialog.addEventListener('click', (event) => {
    if (event.target === nodes.dialog) {
      close();
    }
  });
  nodes.copy.addEventListener('click', async () => {
    await window.telinha.copyText(logLines.join('\n'));
    nodes.copy.textContent = 'Copiado';
    setTimeout(() => {
      nodes.copy.textContent = 'Copiar registro';
    }, 1500);
  });

  function samples(key) {
    const source = sources.get(key);
    return source ? source.samples.slice() : [];
  }

  function logText() {
    return logLines.join('\n');
  }

  return {
    appendLog, close, isOpen, logText, open, quality, remove, samples, update,
  };
})();
