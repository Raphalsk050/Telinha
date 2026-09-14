'use strict';

const Share = (() => {
  const find = (id) => document.getElementById(id);

  const QUALITY_PRESETS = {
    auto: { maxWidth: 0, maxHeight: 0, maxFps: 0, maxBitrateKbps: 0 },
    games: { maxWidth: 2560, maxHeight: 1440, maxFps: 60, maxBitrateKbps: 0 },
    screen: { maxWidth: 0, maxHeight: 0, maxFps: 15, maxBitrateKbps: 0 },
  };
  const PRESET_LABELS = [
    ['auto', 'Automática', 'o Telinha ajusta sozinho'],
    ['games', 'Jogos', '1440p · 60 fps'],
    ['screen', 'Compartilhamento de tela', 'texto nítido · 15 fps'],
    ['custom', 'Personalizada', 'você escolhe'],
  ];
  const RESOLUTIONS = [['0x0', 'Original'], ['2560x1440', '1440p'], ['1920x1080', '1080p'],
    ['1280x720', '720p'], ['854x480', '480p']];
  const FRAMERATES = [['0', 'Automático (até 60)'], ['120', '120'], ['60', '60'], ['30', '30'], ['15', '15'], ['5', '5']];
  const BITRATES = [['0', 'Automática'], ['20000', '20 Mbps'], ['10000', '10 Mbps'], ['6000', '6 Mbps'],
    ['3000', '3 Mbps'], ['1500', '1,5 Mbps']];
  const AUDIO_LABELS = [['system', 'Som do computador'], ['process', 'Só o som do programa'],
    ['device', 'Som da placa'], ['none', 'Sem som']];
  const KIND_BY_TAB = { monitors: 'monitor', windows: 'window', devices: 'device' };
  const EMPTY_TEXT = {
    monitors: 'Nada encontrado aqui.',
    windows: 'Nada encontrado aqui.',
    devices: 'Nenhuma placa de captura conectada. Ligue a placa e toque em Atualizar.',
  };

  const ICONS = {
    monitor: '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="12" rx="2" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M8 20h8M12 16v4" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>',
    window: '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="2" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M3 9h18" stroke="currentColor" stroke-width="1.8"/><circle cx="6" cy="7" r=".9" fill="currentColor"/><circle cx="8.8" cy="7" r=".9" fill="currentColor"/></svg>',
    device: '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="7" width="18" height="10" rx="2" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M7 11h5M7 14h3" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/><circle cx="16.5" cy="12" r="1.6" fill="currentColor"/></svg>',
  };

  const nodes = {
    dialog: find('share-dialog'),
    title: find('share-title'),
    close: find('share-close'),
    tabs: [...document.querySelectorAll('#share-dialog .tab')],
    filter: find('share-filter'),
    refresh: find('share-refresh'),
    status: find('share-status'),
    grid: find('share-grid'),
    settings: find('share-settings'),
    cancel: find('share-cancel'),
    confirm: find('share-confirm'),
    streamDialog: find('stream-dialog'),
    streamClose: find('stream-close'),
    streamFeedback: find('stream-feedback'),
  };

  const picker = { options: null, tab: 'monitors', targets: emptyTargets(), selection: null, token: 0 };
  let stream = null;

  function emptyTargets() {
    return { monitors: [], windows: [], devices: [], audioInputs: [] };
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

  function checkedValue(name) {
    const input = document.querySelector(`input[name="${name}"]:checked`);
    return input ? input.value : null;
  }

  function selectField(id, label, options, onChange) {
    const field = make('label', 'field');
    const select = make('select', 'input select');
    select.id = id;
    for (const [value, text] of options) {
      const option = make('option', null, text);
      option.value = value;
      select.append(option);
    }
    select.addEventListener('change', onChange);
    field.append(make('span', null, label), select);
    return field;
  }

  function buildQuality(container, prefix, onChange) {
    const custom = make('div', 'custom-grid');
    custom.id = `${prefix}-custom`;
    custom.hidden = true;
    const grid = make('div', 'preset-grid');
    grid.setAttribute('role', 'radiogroup');
    for (const [value, name, meta] of PRESET_LABELS) {
      const label = make('label', 'preset');
      const input = make('input');
      input.type = 'radio';
      input.name = `${prefix}-preset`;
      input.value = value;
      input.checked = value === 'auto';
      input.addEventListener('change', () => {
        custom.hidden = value !== 'custom';
        onChange();
      });
      label.append(input, make('span', 'preset-name', name), make('span', 'preset-meta', meta));
      grid.append(label);
    }
    custom.append(
      selectField(`${prefix}-resolution`, 'Resolução', RESOLUTIONS, onChange),
      selectField(`${prefix}-fps`, 'Quadros por segundo', FRAMERATES, onChange),
      selectField(`${prefix}-bitrate`, 'Taxa máxima', BITRATES, onChange),
    );
    container.replaceChildren(make('h4', 'section-title', 'Qualidade da transmissão'), grid, custom);
  }

  function readQuality(prefix) {
    const preset = checkedValue(`${prefix}-preset`) ?? 'auto';
    if (preset !== 'custom') {
      return { preset, ...QUALITY_PRESETS[preset] };
    }
    const [maxWidth, maxHeight] = find(`${prefix}-resolution`).value.split('x').map(Number);
    return {
      preset,
      maxWidth,
      maxHeight,
      maxFps: Number(find(`${prefix}-fps`).value),
      maxBitrateKbps: Number(find(`${prefix}-bitrate`).value),
    };
  }

  function writeQuality(prefix, quality = { preset: 'auto' }) {
    const preset = quality.preset ?? 'auto';
    for (const input of document.querySelectorAll(`input[name="${prefix}-preset"]`)) {
      input.checked = input.value === preset;
    }
    find(`${prefix}-custom`).hidden = preset !== 'custom';
    if (preset === 'custom') {
      const resolution = `${quality.maxWidth}x${quality.maxHeight}`;
      const select = find(`${prefix}-resolution`);
      select.value = [...select.options].some((option) => option.value === resolution) ? resolution : '0x0';
      find(`${prefix}-fps`).value = String(quality.maxFps ?? 0);
      find(`${prefix}-bitrate`).value = String(quality.maxBitrateKbps ?? 0);
    }
  }

  function buildAudio(container, prefix, onChange) {
    const group = make('div', 'segmented');
    group.setAttribute('role', 'radiogroup');
    for (const [value, text] of AUDIO_LABELS) {
      const label = make('label');
      const input = make('input');
      input.type = 'radio';
      input.name = `${prefix}-audio`;
      input.value = value;
      input.checked = value === 'system';
      input.addEventListener('change', () => {
        find(`${prefix}-audio-device-field`).hidden = readAudio(prefix) !== 'device';
        onChange();
      });
      label.append(input, make('span', null, text));
      group.append(label);
    }
    const field = make('label', 'field');
    field.id = `${prefix}-audio-device-field`;
    field.hidden = true;
    const select = make('select', 'input select');
    select.id = `${prefix}-audio-device`;
    select.addEventListener('change', onChange);
    field.append(make('span', null, 'Entrada de som da placa'), select);
    const hint = make('p', 'muted small', 'Para mandar só o som de um programa, compartilhe um programa em Aplicativos.');
    hint.id = `${prefix}-audio-hint`;
    container.replaceChildren(make('h4', 'section-title', 'Som da transmissão'), group, field, hint);
  }

  function readAudio(prefix) {
    return checkedValue(`${prefix}-audio`) ?? 'system';
  }

  function readAudioDevice(prefix) {
    return find(`${prefix}-audio-device`).value || '';
  }

  function writeAudio(prefix, scope, {
    processAvailable = false, deviceAvailable = false, inputs = [], device = '',
  } = {}) {
    const unavailable = (scope === 'process' && !processAvailable)
      || (scope === 'device' && (!deviceAvailable || inputs.length === 0));
    const wanted = unavailable ? 'system' : scope;
    for (const input of document.querySelectorAll(`input[name="${prefix}-audio"]`)) {
      if (input.value === 'process') {
        input.disabled = !processAvailable;
        input.parentElement.hidden = deviceAvailable;
      } else if (input.value === 'device') {
        input.disabled = inputs.length === 0;
        input.parentElement.hidden = !deviceAvailable;
      }
      input.checked = input.value === wanted;
    }

    const select = find(`${prefix}-audio-device`);
    const current = device || select.value;
    select.replaceChildren(...inputs.map((item) => {
      const option = make('option', null, item.name || 'Entrada sem nome');
      option.value = item.id;
      return option;
    }));
    if (current && inputs.some((item) => item.id === current)) {
      select.value = current;
    }
    find(`${prefix}-audio-device-field`).hidden = wanted !== 'device';
    find(`${prefix}-audio-hint`).hidden = processAvailable || deviceAvailable;
  }

  function targetsFor(kind) {
    if (kind === 'monitor') {
      return picker.targets.monitors;
    }
    return kind === 'window' ? picker.targets.windows : picker.targets.devices;
  }

  function targetMeta(kind, target) {
    if (kind === 'monitor') {
      return `${target.width}×${target.height} · ${Math.round(target.refresh_hz)} Hz`;
    }
    if (kind === 'device') {
      return target.audio_name ? `som: ${target.audio_name}` : 'placa de captura';
    }
    return `${target.width}×${target.height}`;
  }

  function targetCard(kind, target) {
    const card = make('button', 'target-card');
    card.type = 'button';
    const selected = Boolean(picker.selection && picker.selection.kind === kind
      && picker.selection.handle === target.handle);
    card.setAttribute('aria-pressed', String(selected));

    const thumb = make('span', 'target-thumb');
    thumb.innerHTML = ICONS[kind];
    const fallback = { monitor: 'Tela sem nome', window: '(sem título)', device: 'Placa de captura' }[kind];
    const name = make('span', 'target-name', target.name || fallback);
    name.title = target.name || fallback;
    if (kind === 'monitor' && target.primary) {
      name.append(make('span', 'badge', 'principal'));
    }

    card.append(thumb, name, make('span', 'target-meta', targetMeta(kind, target)));
    card.addEventListener('click', () => {
      picker.selection = {
        kind,
        handle: target.handle,
        index: target.index,
        pid: target.pid ?? 0,
        name: target.name || fallback,
        audioId: target.audio_id ?? '',
      };
      renderGrid();
      syncControls();
    });
    card.addEventListener('dblclick', confirm);
    return card;
  }

  function renderGrid() {
    const filter = nodes.filter.value.trim().toLowerCase();
    const kind = KIND_BY_TAB[picker.tab];
    const list = kind === 'window'
      ? picker.targets.windows.filter((window) => !filter || window.name.toLowerCase().includes(filter))
      : targetsFor(kind);
    const cards = list.map((target) => targetCard(kind, target));
    if (cards.length === 0) {
      const empty = kind === 'window' && filter ? 'Nenhum programa com esse nome.' : EMPTY_TEXT[picker.tab];
      cards.push(make('p', 'muted small', empty));
    }
    nodes.grid.replaceChildren(...cards);
  }

  function setTab(tab) {
    picker.tab = tab;
    for (const button of nodes.tabs) {
      button.setAttribute('aria-selected', String(button.dataset.tab === tab));
    }
    nodes.filter.hidden = tab !== 'windows';
    renderGrid();
  }

  function syncControls() {
    nodes.confirm.disabled = !picker.selection;
    if (!picker.options || !picker.options.settings) {
      return;
    }
    const selection = picker.selection;
    const onDevice = Boolean(selection && selection.kind === 'device');
    const current = readAudio('share');
    const scope = onDevice && selection.audioId && current !== 'none' ? 'device' : current;
    writeAudio('share', scope, {
      processAvailable: Boolean(selection && selection.kind === 'window' && selection.pid > 0),
      deviceAvailable: onDevice,
      inputs: picker.targets.audioInputs,
      device: onDevice && selection.audioId ? selection.audioId : readAudioDevice('share'),
    });
  }

  async function loadTargets() {
    const token = ++picker.token;
    nodes.status.textContent = 'Procurando telas, programas e placas de captura…';
    nodes.refresh.disabled = true;
    try {
      const targets = await window.telinha.listTargets();
      if (token !== picker.token) {
        return;
      }
      picker.targets = {
        monitors: targets.monitors ?? [],
        windows: targets.windows ?? [],
        devices: targets.devices ?? [],
        audioInputs: targets.audioInputs ?? [],
      };
      const total = picker.targets.monitors.length + picker.targets.windows.length + picker.targets.devices.length;
      nodes.status.textContent = total === 0
        ? 'Não encontrei nada para compartilhar nesta máquina.'
        : 'Escolha o que mostrar.';
    } catch (error) {
      if (token !== picker.token) {
        return;
      }
      picker.targets = emptyTargets();
      nodes.status.textContent = `Não consegui listar as telas: ${error.message}`;
    } finally {
      if (token === picker.token) {
        nodes.refresh.disabled = false;
      }
    }

    if (picker.selection
      && !targetsFor(picker.selection.kind).some((target) => target.handle === picker.selection.handle)) {
      picker.selection = null;
    }
    renderGrid();
    syncControls();
  }

  function openPicker(options) {
    picker.options = options;
    picker.selection = null;
    picker.targets = emptyTargets();
    nodes.title.textContent = options.title;
    nodes.confirm.textContent = options.confirmLabel;
    nodes.settings.hidden = !options.settings;
    if (options.settings) {
      writeQuality('share', options.quality);
      writeAudio('share', options.audio ?? 'system');
    }
    nodes.filter.value = '';
    setTab(picker.tab);
    nodes.dialog.hidden = false;
    syncControls();
    loadTargets();
  }

  function closePicker({ cancelled = false } = {}) {
    const options = picker.options;
    if (!options) {
      return;
    }
    picker.options = null;
    picker.token += 1;
    nodes.dialog.hidden = true;
    if (cancelled && options.onCancel) {
      options.onCancel();
    }
  }

  function confirm() {
    const options = picker.options;
    if (!options || !picker.selection) {
      return;
    }
    const choice = {
      selection: { ...picker.selection },
      quality: options.settings ? readQuality('share') : null,
      audio: options.settings ? readAudio('share') : null,
      audioDevice: options.settings ? readAudioDevice('share') : '',
      audioInputs: picker.targets.audioInputs,
    };
    closePicker();
    options.onConfirm(choice);
  }

  function pickerMode() {
    return picker.options ? picker.options.mode : null;
  }

  function streamAudio(options) {
    return {
      processAvailable: options.processPid > 0,
      deviceAvailable: Boolean(options.deviceShare),
      inputs: options.audioInputs ?? [],
      device: options.audio.device ?? '',
    };
  }

  function openStream(options) {
    stream = options;
    writeQuality('stream', options.quality);
    writeAudio('stream', options.audio.scope, streamAudio(options));
    nodes.streamFeedback.textContent = '';
    nodes.streamDialog.hidden = false;
  }

  function syncStream(options) {
    if (!stream) {
      return;
    }
    stream = { ...stream, ...options };
    writeAudio('stream', stream.audio.scope, streamAudio(stream));
  }

  function streamFeedback(text) {
    nodes.streamFeedback.textContent = text;
  }

  function closeStream() {
    stream = null;
    nodes.streamDialog.hidden = true;
  }

  buildQuality(find('share-quality'), 'share', () => {});
  buildAudio(find('share-audio'), 'share', () => {});
  buildQuality(find('stream-quality'), 'stream', () => {
    if (stream) {
      stream.onQuality(readQuality('stream'));
    }
  });
  buildAudio(find('stream-audio'), 'stream', () => {
    if (stream) {
      const scope = readAudio('stream');
      stream.onAudio(scope, scope === 'process' ? stream.processPid : 0,
        scope === 'device' ? readAudioDevice('stream') : '');
    }
  });

  for (const button of nodes.tabs) {
    button.addEventListener('click', () => setTab(button.dataset.tab));
  }
  nodes.filter.addEventListener('input', renderGrid);
  nodes.refresh.addEventListener('click', loadTargets);
  nodes.confirm.addEventListener('click', confirm);
  nodes.cancel.addEventListener('click', () => closePicker({ cancelled: true }));
  nodes.close.addEventListener('click', () => closePicker({ cancelled: true }));
  nodes.dialog.addEventListener('click', (event) => {
    if (event.target === nodes.dialog) {
      closePicker({ cancelled: true });
    }
  });
  nodes.streamClose.addEventListener('click', closeStream);
  nodes.streamDialog.addEventListener('click', (event) => {
    if (event.target === nodes.streamDialog) {
      closeStream();
    }
  });

  return {
    QUALITY_PRESETS, closePicker, closeStream, openPicker, openStream, pickerMode, streamFeedback, syncStream,
  };
})();
