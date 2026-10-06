'use strict';

// Estilo de perfil de cada pessoa: o efeito do nome, as cores do cartao e o "sobre mim". O de cada
// um chega pela presenca e fica guardado, para o nome continuar estilizado no historico com a
// pessoa offline.
const NameStyle = (() => {
  const STORAGE_KEY = 'telinha.styles';
  const MAX_REMEMBERED = 300;
  const SAVE_DELAY_MS = 400;
  // Quantas cores cada efeito leva, no minimo e no maximo. O mesmo esta em src/profile.js.
  const EFFECTS = {
    solid: [1, 1], gradient: [2, 2], neon: [1, 1], prism: [2, 5],
  };
  const DEFAULT_COLORS = ['#5865f2', '#eb459e', '#19b8b0', '#f0b232', '#23a55a'];
  const EMPTY = { name: null, theme: null, bio: '' };

  const known = load();
  let editor = null;
  let draft = { ...EMPTY };
  let saveTimer = null;

  function load() {
    try {
      return new Map(Object.entries(JSON.parse(localStorage.getItem(STORAGE_KEY)) ?? {}));
    } catch {
      return new Map();
    }
  }

  function persist() {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(Object.fromEntries(known)));
    } catch {
      // sem armazenamento local, os estilos valem so nesta execucao
    }
  }

  function whole(style) {
    return { name: style?.name ?? null, theme: style?.theme ?? null, bio: style?.bio ?? '' };
  }

  // Devolve true quando o estilo guardado da pessoa mudou.
  function remember(memberId, style) {
    if (!memberId) {
      return false;
    }
    const next = whole(style);
    const kept = next.name || next.theme || next.bio ? next : null;
    const before = known.get(memberId) ?? null;
    if (JSON.stringify(before) === JSON.stringify(kept)) {
      return false;
    }
    known.delete(memberId);
    if (kept) {
      known.set(memberId, kept);
    }
    while (known.size > MAX_REMEMBERED) {
      known.delete(known.keys().next().value);
    }
    persist();
    return true;
  }

  function of(memberId) {
    return memberId ? known.get(memberId) ?? null : null;
  }

  function paint(element, style, moving = false) {
    element.classList.remove('name-moving', ...Object.keys(EFFECTS).map((effect) => `name-${effect}`));
    element.style.removeProperty('--name-color');
    element.style.removeProperty('--name-flow');
    const name = style ? style.name : null;
    if (!name || !EFFECTS[name.effect]) {
      return;
    }
    element.classList.add(`name-${name.effect}`);
    element.classList.toggle('name-moving', moving);
    element.style.setProperty('--name-color', name.colors[0]);
    if (name.colors.length > 1) {
      // A primeira cor volta no fim para o degrade emendar quando corre.
      element.style.setProperty('--name-flow', `linear-gradient(90deg, ${[...name.colors, name.colors[0]].join(', ')})`);
    }
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

  /* cartao de perfil */

  // person: { name, activity, avatar (elemento), banner (endereco ou null), onEdit (so no proprio) }
  function fillCard(card, person, style) {
    const theme = style ? style.theme : null;
    card.classList.toggle('themed', Boolean(theme));
    for (const [name, value] of [['--card-a', theme?.[0]], ['--card-b', theme?.[1]]]) {
      if (value) {
        card.style.setProperty(name, value);
      } else {
        card.style.removeProperty(name);
      }
    }

    const banner = make('div', 'profile-card-banner');
    if (person.banner) {
      const image = make('img');
      image.src = person.banner;
      image.alt = '';
      banner.classList.add('has-image');
      banner.append(image);
    }
    const avatar = make('div', 'profile-card-avatar');
    avatar.append(person.avatar);

    const name = make('strong', 'profile-card-name', person.name);
    paint(name, style, true);
    const body = make('div', 'profile-card-body');
    body.append(name);
    if (person.activity) {
      body.append(make('span', 'profile-card-activity', person.activity));
    }
    if (style && style.bio) {
      const about = make('div', 'profile-card-section');
      about.append(make('span', 'profile-card-label', 'Sobre mim'), make('p', 'profile-card-bio', style.bio));
      body.append(about);
    }
    if (person.onEdit) {
      const edit = make('button', 'button profile-card-edit', 'Editar perfil');
      edit.type = 'button';
      edit.addEventListener('click', () => {
        closeCard();
        person.onEdit();
      });
      body.append(edit);
    }

    const inner = make('div', 'profile-card-inner');
    inner.append(banner, avatar, body);
    card.replaceChildren(inner);
  }

  function closeCard() {
    const card = document.getElementById('profile-card');
    if (card) {
      card.hidden = true;
    }
  }

  // Abre ao lado de onde a pessoa clicou, sem sair da janela.
  function openCard(event, person) {
    const card = document.getElementById('profile-card');
    fillCard(card, person, of(person.memberId));
    card.hidden = false;
    const { width, height } = card.getBoundingClientRect();
    const node = person.anchor ?? event.currentTarget;
    const anchor = node instanceof Element
      ? node.getBoundingClientRect()
      : { left: event.clientX, right: event.clientX, top: event.clientY };
    // Quem esta na metade direita da janela, como a lista de membros, abre o cartao para a esquerda.
    const onRight = (anchor.left + anchor.right) / 2 > window.innerWidth / 2;
    const wanted = onRight ? anchor.left - width - 12 : anchor.right + 12;
    card.style.left = `${Math.max(8, Math.min(wanted, window.innerWidth - width - 8))}px`;
    card.style.top = `${Math.max(8, Math.min(anchor.top - 24, window.innerHeight - height - 8))}px`;
  }

  /* editor nas configuracoes */

  function find(id) {
    return document.getElementById(id);
  }

  function colorField(value, label, onInput, onRemove) {
    const field = make('span', 'custom-color');
    const input = make('input');
    input.type = 'color';
    input.value = value;
    input.setAttribute('aria-label', label);
    input.addEventListener('input', () => onInput(input.value));
    field.append(input);
    if (onRemove) {
      const remove = make('button', 'custom-color-remove', '×');
      remove.type = 'button';
      remove.title = 'Tirar esta cor';
      remove.setAttribute('aria-label', `Tirar ${label.toLowerCase()}`);
      remove.addEventListener('click', onRemove);
      field.append(remove);
    }
    return field;
  }

  // Enquanto a pessoa arrasta dentro do seletor de cor, trocar os campos fecharia o seletor.
  function showColors(list, fields) {
    const editing = list.contains(document.activeElement) && document.activeElement.type === 'color'
      && list.children.length === fields.length;
    if (!editing) {
      list.replaceChildren(...fields);
    }
  }

  function change(next) {
    draft = { ...draft, ...next };
    showEditor();
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      saveTimer = null;
      editor.save(draft);
    }, SAVE_DELAY_MS);
  }

  function setNameColors(colors) {
    change({ name: { ...draft.name, colors } });
  }

  function chooseEffect(effect) {
    if (!EFFECTS[effect]) {
      change({ name: null });
      return;
    }
    const [least, most] = EFFECTS[effect];
    const before = draft.name ? draft.name.colors : [];
    const count = Math.max(least, Math.min(most, before.length || least));
    change({ name: { effect, colors: [...before, ...DEFAULT_COLORS].slice(0, count) } });
  }

  function showEditor() {
    const { name, theme, bio } = draft;
    for (const input of document.querySelectorAll('input[name="profile-name-effect"]')) {
      input.checked = input.value === (name ? name.effect : 'none');
    }
    const [least, most] = name ? EFFECTS[name.effect] : [0, 0];
    const colors = name ? name.colors : [];
    showColors(find('profile-name-colors'), colors.map((color, index) => colorField(
      color,
      `Cor ${index + 1} do nome`,
      (value) => setNameColors(draft.name.colors.map((item, at) => (at === index ? value : item))),
      colors.length > least ? () => setNameColors(draft.name.colors.filter((_item, at) => at !== index)) : null,
    )));
    find('profile-name-add').hidden = colors.length >= most;

    find('profile-theme-on').checked = Boolean(theme);
    showColors(find('profile-theme-colors'), (theme ?? []).map((color, index) => colorField(
      color,
      `Cor ${index + 1} do cartão`,
      (value) => change({ theme: draft.theme.map((item, at) => (at === index ? value : item)) }),
      null,
    )));

    const about = find('profile-bio');
    if (document.activeElement !== about) {
      about.value = bio;
    }
    const person = editor.person();
    find('profile-banner-remove').hidden = !person.banner;
    fillCard(find('profile-preview'), person, draft);
  }

  // Chamado quando o perfil chega ou muda, para os controles mostrarem o que esta guardado.
  function refreshEditor() {
    if (!editor) {
      return;
    }
    // Com uma mudanca ainda por salvar, o que esta na tela e mais novo que o perfil guardado.
    if (saveTimer === null) {
      draft = whole(editor.style());
    }
    showEditor();
  }

  function bindEditor(options) {
    editor = options;
    for (const input of document.querySelectorAll('input[name="profile-name-effect"]')) {
      input.addEventListener('change', () => chooseEffect(input.value));
    }
    find('profile-name-add').addEventListener('click', () => {
      const { colors } = draft.name;
      setNameColors([...colors, DEFAULT_COLORS[colors.length % DEFAULT_COLORS.length]]);
    });
    find('profile-theme-on').addEventListener('change', (event) => {
      change({ theme: event.target.checked ? DEFAULT_COLORS.slice(0, 2) : null });
    });
    find('profile-bio').addEventListener('input', (event) => change({ bio: event.target.value }));

    const picker = find('profile-banner-file');
    find('profile-banner-change').addEventListener('click', () => picker.click());
    picker.addEventListener('change', () => {
      const [file] = picker.files;
      picker.value = '';
      if (file) {
        editor.cropBanner(file);
      }
    });
    find('profile-banner-remove').addEventListener('click', () => editor.removeBanner());

    document.addEventListener('mousedown', (event) => {
      const card = find('profile-card');
      if (!card.hidden && !card.contains(event.target)) {
        closeCard();
      }
    }, true);
    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') {
        closeCard();
      }
    });
    refreshEditor();
  }

  return {
    remember, of, paint, openCard, closeCard, bindEditor, refreshEditor,
  };
})();
