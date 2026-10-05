'use strict';

// Aparencia do app: tema, densidade, jeito das mensagens e tamanhos. Carregado no <head> para o
// tema valer antes da primeira pintura. As cores de cada tema estao no comeco de styles.css.
const Appearance = (() => {
  const STORAGE_KEY = 'telinha.appearance';
  const CUSTOM = 'custom';
  const MAX_COLORS = 5;
  const HEX_PATTERN = /^#[0-9a-f]{6}$/i;

  // Temas de cor: um degrade atras do app inteiro, sobre a base escura ou clara.
  const COLOR_THEMES = [
    { id: 'aurora', name: 'Aurora', base: 'dark', angle: 135, colors: ['#0b3d4f', '#1f8a70', '#7b2ff7'] },
    { id: 'sunset', name: 'Pôr do sol', base: 'dark', angle: 160, colors: ['#48217a', '#c2416b', '#f08a4b'] },
    { id: 'ocean', name: 'Oceano', base: 'dark', angle: 150, colors: ['#0a2a66', '#0f6fa8', '#19b8b0'] },
    { id: 'forest', name: 'Floresta', base: 'dark', angle: 140, colors: ['#0f3d2e', '#2f7d4f', '#a6b85a'] },
    { id: 'midnight', name: 'Meia-noite', base: 'dark', angle: 135, colors: ['#1a1446', '#3b3fb5', '#5865f2'] },
    { id: 'crimson', name: 'Lua carmesim', base: 'dark', angle: 150, colors: ['#3a0d18', '#8a1c2b', '#d1495b'] },
    { id: 'neon', name: 'Neon', base: 'dark', angle: 120, colors: ['#3d0f6b', '#c2187a', '#12b3d6'] },
    { id: 'dusk', name: 'Crepúsculo', base: 'dark', angle: 170, colors: ['#2b2f77', '#6b4fa0', '#c47fb0'] },
    { id: 'mint', name: 'Menta', base: 'light', angle: 135, colors: ['#bff0d4', '#e6f7b2'] },
    { id: 'citrus', name: 'Cítrico', base: 'light', angle: 135, colors: ['#ffd9a0', '#ff9aa2'] },
    { id: 'candy', name: 'Algodão-doce', base: 'light', angle: 135, colors: ['#f9c2e3', '#b8d8ff'] },
    { id: 'blossom', name: 'Cerejeira', base: 'light', angle: 150, colors: ['#ffd1dc', '#e8c7f5', '#c9e4ff'] },
    { id: 'sunrise', name: 'Nascer do sol', base: 'light', angle: 160, colors: ['#ffe29a', '#ffa99f', '#c5b3ff'] },
    { id: 'sepia', name: 'Sépia', base: 'light', angle: 135, colors: ['#e8d8bf', '#cbb393'] },
  ];
  const PRESET_INTENSITY = 75;

  const DEFAULTS = {
    theme: 'ash',
    darkSidebar: false,
    motion: false,
    density: 'default',
    messages: 'cozy',
    chatFont: 15.5,
    messageGap: 16,
    zoom: 100,
    custom: { base: 'dark', colors: ['#5865f2', '#eb459e'], angle: 135, intensity: 50 },
  };
  const CHOICES = {
    theme: ['light', 'ash', 'dark', 'onyx', 'system', CUSTOM, ...COLOR_THEMES.map((theme) => `color:${theme.id}`)],
    density: ['compact', 'default', 'spacious'],
    messages: ['cozy', 'compact'],
  };
  const RANGES = { chatFont: [12, 24], messageGap: [0, 24], zoom: [50, 200] };
  const CUSTOM_RANGES = { angle: [0, 360], intensity: [0, 100] };
  const SLIDERS = {
    chatFont: ['appearance-chat-font', (value) => `${value.toLocaleString('pt-BR')} px`],
    messageGap: ['appearance-message-gap', (value) => `${value} px`],
    zoom: ['appearance-zoom', (value) => `${value}%`],
  };
  const CUSTOM_SLIDERS = {
    angle: ['appearance-custom-angle', (value) => `${value}°`],
    intensity: ['appearance-custom-intensity', (value) => `${value}%`],
  };
  // O veu e a cor da base por cima do degrade. Quanto mais intenso o tema, mais fino o veu, ate o
  // ponto em que o texto ainda se le sobre a cor mais forte do degrade.
  const VEIL = {
    dark: { color: '12 12 14', hover: '255 255 255 / 0.08', selected: '255 255 255 / 0.14', line: '0 0 0 / 0.35', soft: '255 255 255 / 0.14' },
    light: { color: '255 255 255', hover: '0 0 0 / 0.06', selected: '0 0 0 / 0.1', line: '0 0 0 / 0.12', soft: '0 0 0 / 0.18' },
  };

  const lightScheme = window.matchMedia('(prefers-color-scheme: light)');
  let settings = load();

  function clamp(value, [low, high]) {
    return Math.min(high, Math.max(low, value));
  }

  function cleanCustom(candidate) {
    const source = candidate && typeof candidate === 'object' ? candidate : {};
    const colors = (Array.isArray(source.colors) ? source.colors : [])
      .filter((color) => HEX_PATTERN.test(color))
      .slice(0, MAX_COLORS)
      .map((color) => color.toLowerCase());
    const result = {
      base: source.base === 'light' ? 'light' : 'dark',
      colors: colors.length > 0 ? colors : [...DEFAULTS.custom.colors],
      angle: DEFAULTS.custom.angle,
      intensity: DEFAULTS.custom.intensity,
    };
    for (const [key, range] of Object.entries(CUSTOM_RANGES)) {
      const value = Number(source[key]);
      if (Number.isFinite(value)) {
        result[key] = clamp(value, range);
      }
    }
    return result;
  }

  function clean(candidate) {
    const result = { ...DEFAULTS };
    const source = candidate && typeof candidate === 'object' ? candidate : {};
    for (const [key, allowed] of Object.entries(CHOICES)) {
      if (allowed.includes(source[key])) {
        result[key] = source[key];
      }
    }
    for (const [key, range] of Object.entries(RANGES)) {
      const value = Number(source[key]);
      if (Number.isFinite(value)) {
        result[key] = clamp(value, range);
      }
    }
    result.darkSidebar = source.darkSidebar === true;
    result.motion = source.motion === true;
    result.custom = cleanCustom(source.custom);
    return result;
  }

  function load() {
    try {
      return clean(JSON.parse(localStorage.getItem(STORAGE_KEY)));
    } catch {
      return clean(null);
    }
  }

  function save() {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
    } catch {
      // sem armazenamento local, a escolha vale so nesta execucao
    }
  }

  function gradientOf(current) {
    if (current.theme === CUSTOM) {
      return current.custom;
    }
    const preset = COLOR_THEMES.find((theme) => `color:${theme.id}` === current.theme);
    return preset ? { ...preset, intensity: PRESET_INTENSITY } : null;
  }

  function channels(hex) {
    return [1, 3, 5].map((start) => Number.parseInt(hex.slice(start, start + 2), 16));
  }

  function luminance(hex) {
    const [red, green, blue] = channels(hex).map((value) => {
      const unit = value / 255;
      return unit <= 0.03928 ? unit / 12.92 : ((unit + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
  }

  function cssGradient({ colors, angle }) {
    const stops = colors.length > 1 ? colors : [colors[0], colors[0]];
    return `linear-gradient(${angle}deg, ${stops.join(', ')})`;
  }

  function applyGradient(root, gradient) {
    const names = ['--gradient', '--tint', '--veil-main', '--veil-side', '--veil-rail', '--veil-hover',
      '--veil-selected', '--veil-line', '--veil-line-soft'];
    root.toggleAttribute('data-gradient', Boolean(gradient));
    root.toggleAttribute('data-gradient-motion', Boolean(gradient) && settings.motion);
    if (!gradient) {
      for (const name of names) {
        root.style.removeProperty(name);
      }
      return;
    }

    const veil = VEIL[gradient.base];
    const lights = gradient.colors.map(luminance);
    // Na base escura a cor mais clara e a que mais atrapalha o texto, e na clara, a mais escura.
    const harshest = gradient.base === 'dark' ? Math.max(...lights) : 1 - Math.min(...lights);
    const readable = 0.5 + 0.38 * harshest;
    const alpha = Math.max(0.9 - 0.4 * (gradient.intensity / 100), readable);
    const mixed = [0, 1, 2].map((index) => Math.round(
      gradient.colors.reduce((total, color) => total + channels(color)[index], 0) / gradient.colors.length,
    ));

    root.style.setProperty('--gradient', cssGradient(gradient));
    root.style.setProperty('--tint', `rgb(${mixed.join(' ')})`);
    root.style.setProperty('--veil-main', `rgb(${veil.color} / ${alpha.toFixed(3)})`);
    root.style.setProperty('--veil-side', `rgb(${veil.color} / ${Math.min(0.94, alpha + 0.07).toFixed(3)})`);
    root.style.setProperty('--veil-rail', `rgb(${veil.color} / ${Math.min(0.96, alpha + 0.14).toFixed(3)})`);
    root.style.setProperty('--veil-hover', `rgb(${veil.hover})`);
    root.style.setProperty('--veil-selected', `rgb(${veil.selected})`);
    root.style.setProperty('--veil-line', `rgb(${veil.line})`);
    root.style.setProperty('--veil-line-soft', `rgb(${veil.soft})`);
  }

  function apply() {
    const root = document.documentElement;
    const gradient = gradientOf(settings);
    const followed = lightScheme.matches ? 'light' : DEFAULTS.theme;
    const plain = settings.theme === 'system' ? followed : settings.theme;
    root.dataset.theme = gradient ? gradient.base : plain;
    root.toggleAttribute('data-dark-sidebar', settings.darkSidebar && !gradient);
    root.dataset.density = settings.density;
    root.dataset.messages = settings.messages;
    root.style.setProperty('--chat-font', `${settings.chatFont}px`);
    root.style.setProperty('--message-gap', `${settings.messageGap}px`);
    applyGradient(root, gradient);
  }

  function applyZoom() {
    if (window.telinha && window.telinha.setZoom) {
      window.telinha.setZoom(settings.zoom / 100);
    }
  }

  function find(id) {
    return document.getElementById(id);
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

  function showSlider([id, describe], value) {
    find(id).value = String(value);
    find(`${id}-value`).textContent = describe(value);
  }

  function setCustom(changes) {
    set({ theme: CUSTOM, custom: { ...settings.custom, ...changes } });
  }

  function showCustomColors() {
    const { colors } = settings.custom;
    const rows = colors.map((color, index) => {
      const row = make('span', 'custom-color');
      const input = make('input');
      input.type = 'color';
      input.value = color;
      input.setAttribute('aria-label', `Cor ${index + 1}`);
      input.addEventListener('input', () => {
        setCustom({ colors: settings.custom.colors.map((item, at) => (at === index ? input.value : item)) });
      });
      row.append(input);
      if (colors.length > 1) {
        const remove = make('button', 'custom-color-remove', '×');
        remove.type = 'button';
        remove.title = 'Tirar esta cor';
        remove.setAttribute('aria-label', `Tirar a cor ${index + 1}`);
        remove.addEventListener('click', () => {
          setCustom({ colors: settings.custom.colors.filter((_item, at) => at !== index) });
        });
        row.append(remove);
      }
      return row;
    });
    const list = find('appearance-custom-colors');
    // Enquanto a pessoa arrasta dentro do seletor de cor, trocar o campo fecharia o seletor.
    const editing = list.contains(document.activeElement) && document.activeElement.type === 'color'
      && list.children.length === rows.length;
    if (editing) {
      [...list.querySelectorAll('input')].forEach((input, index) => {
        if (input !== document.activeElement) {
          input.value = colors[index];
        }
      });
    } else {
      list.replaceChildren(...rows);
    }
    find('appearance-custom-add').hidden = colors.length >= MAX_COLORS;
    find('appearance-custom-angle-row').hidden = colors.length < 2;
  }

  function showControls() {
    for (const key of Object.keys(CHOICES)) {
      for (const input of document.querySelectorAll(`input[name="appearance-${key}"]`)) {
        input.checked = input.value === settings[key];
      }
    }
    const gradient = gradientOf(settings);
    find('appearance-dark-sidebar').checked = settings.darkSidebar;
    find('appearance-dark-sidebar-row').hidden = settings.theme !== 'light' && settings.theme !== 'system';
    find('appearance-motion').checked = settings.motion;
    find('appearance-motion-row').hidden = !gradient;
    for (const key of Object.keys(SLIDERS)) {
      showSlider(SLIDERS[key], settings[key]);
    }

    find('appearance-custom').hidden = settings.theme !== CUSTOM;
    find('appearance-custom-dot').style.background = cssGradient(settings.custom);
    for (const input of document.querySelectorAll('input[name="appearance-custom-base"]')) {
      input.checked = input.value === settings.custom.base;
    }
    for (const key of Object.keys(CUSTOM_SLIDERS)) {
      showSlider(CUSTOM_SLIDERS[key], settings.custom[key]);
    }
    showCustomColors();
  }

  function set(changes) {
    const zoomBefore = settings.zoom;
    settings = clean({ ...settings, ...changes });
    save();
    apply();
    if (settings.zoom !== zoomBefore) {
      applyZoom();
    }
    showControls();
  }

  function randomColor(base) {
    const hue = Math.floor(Math.random() * 360);
    const saturation = 55 + Math.floor(Math.random() * 35);
    const lightness = base === 'dark' ? 22 + Math.floor(Math.random() * 28) : 74 + Math.floor(Math.random() * 14);
    const probe = make('canvas').getContext('2d');
    probe.fillStyle = `hsl(${hue} ${saturation}% ${lightness}%)`;
    return probe.fillStyle;
  }

  function surprise() {
    const { base } = settings.custom;
    const count = 2 + Math.floor(Math.random() * 3);
    setCustom({
      colors: Array.from({ length: count }, () => randomColor(base)),
      angle: Math.floor(Math.random() * 72) * 5,
    });
  }

  function buildColorThemes() {
    const list = find('appearance-color-themes');
    for (const theme of COLOR_THEMES) {
      const label = make('label', 'theme-swatch');
      const input = make('input');
      input.type = 'radio';
      input.name = 'appearance-theme';
      input.value = `color:${theme.id}`;
      const dot = make('span', 'theme-dot');
      dot.style.background = cssGradient(theme);
      label.append(input, dot, make('span', null, theme.name));
      list.append(label);
    }
  }

  function bind() {
    buildColorThemes();
    for (const key of Object.keys(CHOICES)) {
      for (const input of document.querySelectorAll(`input[name="appearance-${key}"]`)) {
        input.addEventListener('change', () => set({ [key]: input.value }));
      }
    }
    find('appearance-dark-sidebar').addEventListener('change', (event) => set({ darkSidebar: event.target.checked }));
    find('appearance-motion').addEventListener('change', (event) => set({ motion: event.target.checked }));
    for (const key of Object.keys(SLIDERS)) {
      const input = find(SLIDERS[key][0]);
      // O zoom so vale ao soltar: aplicar no meio do arraste tiraria o controle de baixo do mouse.
      if (key === 'zoom') {
        input.addEventListener('input', () => showSlider(SLIDERS[key], Number(input.value)));
      } else {
        input.addEventListener('input', () => set({ [key]: Number(input.value) }));
      }
      input.addEventListener('change', () => set({ [key]: Number(input.value) }));
    }

    for (const input of document.querySelectorAll('input[name="appearance-custom-base"]')) {
      input.addEventListener('change', () => setCustom({ base: input.value }));
    }
    for (const key of Object.keys(CUSTOM_SLIDERS)) {
      const input = find(CUSTOM_SLIDERS[key][0]);
      input.addEventListener('input', () => setCustom({ [key]: Number(input.value) }));
    }
    find('appearance-custom-add').addEventListener('click', () => {
      const { colors } = settings.custom;
      setCustom({ colors: [...colors, colors[colors.length - 1]] });
    });
    find('appearance-custom-surprise').addEventListener('click', surprise);
    find('appearance-reset').addEventListener('click', () => set(DEFAULTS));
    showControls();
  }

  lightScheme.addEventListener('change', apply);
  apply();
  document.addEventListener('DOMContentLoaded', () => {
    applyZoom();
    bind();
  });

  return { get: () => structuredClone(settings), set };
})();
