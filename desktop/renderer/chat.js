'use strict';

const Chat = (() => {
  const GROUP_WINDOW_MS = 5 * 60 * 1000;
  const INVITE_PATTERN = /TELINHA-GRUPO\.[A-Za-z0-9_-]+/;
  const LINK_PATTERN = /\b(?:https?:\/\/|www\.)[^\s<>"]+/gi;
  const MAX_IMAGE_BYTES = 380 * 1024;
  const MAX_FILE_BYTES = 1024 * 1024 * 1024;
  const IMAGE_SIDES = [1920, 1600, 1280, 960, 720];
  const IMAGE_QUALITIES = [0.85, 0.72, 0.6];
  const IMAGE_BOUNDS = { width: 400, height: 300 };
  const EMBED_BOUNDS = { width: 400, height: 225 };
  const EMBED_IMAGE_BYTES = 120 * 1024;
  const EMBED_IMAGE_SIDES = [640, 480, 360];
  const EMBED_DELAY_MS = 400;
  const EMBED_WAIT_MS = 2500;
  const IMAGE_CACHE_LIMIT = 80;
  const DOWNLOAD_ICON = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4v11M7 10.5l5 5 5-5M5 20h14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>';
  const FILE_ICON = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 2.5h8l4.5 4.5v14a.5.5 0 0 1-.5.5H6a.5.5 0 0 1-.5-.5V3a.5.5 0 0 1 .5-.5z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/><path d="M14 2.5V7h4.5M8.5 12h7M8.5 15h7M8.5 18h4.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>';
  const find = (id) => document.getElementById(id);

  const nodes = {
    view: find('view-chat'),
    scroll: find('chat-scroll'),
    introAvatar: find('chat-intro-avatar'),
    introTitle: find('chat-intro-title'),
    introText: find('chat-intro-text'),
    messages: find('chat-messages'),
    form: find('chat-form'),
    input: find('chat-input'),
    attach: find('chat-attach'),
    file: find('chat-file'),
    pending: find('chat-pending'),
    pendingImage: find('chat-pending-image'),
    pendingIcon: find('chat-pending-icon'),
    pendingName: find('chat-pending-name'),
    pendingBar: find('chat-pending-bar'),
    pendingRemove: find('chat-pending-remove'),
    embed: find('chat-embed'),
    embedSite: find('chat-embed-site'),
    embedTitle: find('chat-embed-title'),
    embedText: find('chat-embed-text'),
    embedImage: find('chat-embed-image'),
    embedRemove: find('chat-embed-remove'),
    hint: find('chat-hint'),
    lightbox: find('lightbox'),
    lightboxImage: find('lightbox-image'),
  };

  const imageCache = new Map();
  const imageLoads = new Map();
  const progress = new Map();
  const fetches = new Map();
  const fileCards = new Map();
  const dismissedLinks = new Set();

  let conversation = null;
  let messages = [];
  let loadToken = 0;
  let sending = false;
  let preparing = false;
  let attachment = null;
  let uploadId = null;
  let lightboxSource = null;
  let draftEmbed = null;
  let embedTimer = null;
  let profileName = () => 'Você';

  function initial(name) {
    return String(name || '?').charAt(0).toUpperCase();
  }

  function formatTime(timestamp) {
    const date = new Date(timestamp);
    const time = date.toLocaleTimeString('pt-BR', { hour: '2-digit', minute: '2-digit' });
    const today = new Date();
    if (date.toDateString() === today.toDateString()) {
      return `Hoje às ${time}`;
    }
    const yesterday = new Date(today);
    yesterday.setDate(today.getDate() - 1);
    if (date.toDateString() === yesterday.toDateString()) {
      return `Ontem às ${time}`;
    }
    return `${date.toLocaleDateString('pt-BR')} ${time}`;
  }

  function formatBytes(bytes) {
    if (bytes < 1024) {
      return `${bytes} B`;
    }
    if (bytes < 1024 * 1024) {
      return `${Math.round(bytes / 1024)} KB`;
    }
    if (bytes < 1024 * 1024 * 1024) {
      return `${(bytes / (1024 * 1024)).toLocaleString('pt-BR', { maximumFractionDigits: 1 })} MB`;
    }
    return `${(bytes / (1024 * 1024 * 1024)).toLocaleString('pt-BR', { maximumFractionDigits: 2 })} GB`;
  }

  function formatPercent(fraction) {
    return `${Math.floor(Math.min(1, Math.max(0, fraction)) * 100)}%`;
  }

  function setBar(bar, fraction) {
    bar.hidden = fraction === null;
    if (fraction !== null) {
      bar.firstElementChild.style.width = `${Math.min(1, Math.max(0, fraction)) * 100}%`;
    }
  }

  function cleanError(error) {
    return String(error && error.message ? error.message : error)
      .replace(/^Error invoking remote method '[^']+': (Error: )?/, '');
  }

  function delay(milliseconds) {
    return new Promise((resolve) => setTimeout(resolve, milliseconds));
  }

  function authorOf(message) {
    if (message.mine) {
      return profileName();
    }
    return conversation && conversation.authorFor ? conversation.authorFor(message) : message.author;
  }

  function nearBottom() {
    return nodes.scroll.scrollHeight - nodes.scroll.scrollTop - nodes.scroll.clientHeight < 80;
  }

  function scrollToBottom() {
    nodes.scroll.scrollTop = nodes.scroll.scrollHeight;
  }

  /* links */

  function trimLink(raw) {
    let link = raw;
    const unbalanced = (open, closing) => link.endsWith(closing) && link.split(open).length < link.split(closing).length;
    while (link && (/[.,;:!?'"*_~]$/.test(link) || unbalanced('(', ')') || unbalanced('[', ']') || unbalanced('{', '}'))) {
      link = link.slice(0, -1);
    }
    return link;
  }

  function linkTarget(text) {
    const address = /^www\./i.test(text) ? `https://${text}` : text;
    try {
      const url = new URL(address);
      return (url.protocol === 'http:' || url.protocol === 'https:') && url.hostname ? url.href : null;
    } catch {
      return null;
    }
  }

  function splitLinks(text) {
    const parts = [];
    let last = 0;
    for (const match of text.matchAll(LINK_PATTERN)) {
      const raw = trimLink(match[0]);
      const href = raw ? linkTarget(raw) : null;
      if (!href) {
        continue;
      }
      if (match.index > last) {
        parts.push({ text: text.slice(last, match.index) });
      }
      parts.push({ text: raw, href });
      last = match.index + raw.length;
    }
    if (last < text.length) {
      parts.push({ text: text.slice(last) });
    }
    return parts;
  }

  function firstLink(text) {
    const part = splitLinks(text).find((item) => item.href);
    return part ? part.href : null;
  }

  function linkNode(text, href) {
    const link = document.createElement('a');
    link.className = 'message-link';
    link.href = href;
    link.textContent = text;
    link.title = href;
    link.draggable = false;
    return link;
  }

  function appendLinked(parent, text) {
    for (const part of splitLinks(text)) {
      parent.append(part.href ? linkNode(part.text, part.href) : document.createTextNode(part.text));
    }
  }

  function openLink(href) {
    window.telinha.openExternal(href).catch(() => {});
  }

  function openLinkMenu(event, href) {
    event.preventDefault();
    openContextMenu(event, [
      { type: 'action', label: 'Abrir link', onSelect: () => openLink(href) },
      { type: 'action', label: 'Copiar link', onSelect: () => window.telinha.copyText(href) },
    ]);
  }

  /* anexos recebidos */

  function cacheImage(key, url) {
    imageCache.set(key, url);
    if (imageCache.size > IMAGE_CACHE_LIMIT) {
      imageCache.delete(imageCache.keys().next().value);
    }
  }

  function loadImage(target, imageId, key) {
    if (!imageLoads.has(key)) {
      const request = window.telinha.chatImage(target.spaceId, target.channelId, imageId)
        .catch(() => null)
        .then((url) => {
          imageLoads.delete(key);
          if (url) {
            cacheImage(key, url);
          }
          return url;
        });
      imageLoads.set(key, request);
    }
    return imageLoads.get(key);
  }

  function openLightbox(url, source) {
    lightboxSource = source ?? null;
    nodes.lightboxImage.src = url;
    nodes.lightbox.hidden = false;
  }

  function closeLightbox() {
    lightboxSource = null;
    nodes.lightbox.hidden = true;
    nodes.lightboxImage.removeAttribute('src');
  }

  async function saveImage(source) {
    try {
      if (await window.telinha.saveChatImage(source.target.spaceId, source.target.channelId, source.imageId)) {
        nodes.hint.textContent = 'Imagem salva.';
      }
    } catch (error) {
      nodes.hint.textContent = `Não consegui salvar: ${cleanError(error)}`;
    }
  }

  function openImageMenu(event, source) {
    if (!source) {
      return;
    }
    event.preventDefault();
    openContextMenu(event, [{ type: 'action', label: 'Salvar imagem', onSelect: () => saveImage(source) }]);
  }

  function imageNode(image, bounds, className = 'message-image') {
    const frame = document.createElement('button');
    frame.type = 'button';
    frame.className = className;
    frame.title = 'Abrir imagem';
    const width = Math.max(1, Number(image.width) || 1);
    const height = Math.max(1, Number(image.height) || 1);
    const scale = Math.min(1, bounds.width / width, bounds.height / height);
    frame.style.width = `${Math.max(60, Math.round(width * scale))}px`;
    frame.style.height = `${Math.max(60, Math.round(height * scale))}px`;

    const picture = document.createElement('img');
    picture.alt = 'Imagem';
    frame.append(picture);

    const target = conversation;
    const key = `${target.key}/${image.id}`;
    const show = (url) => {
      if (url) {
        picture.src = url;
      } else {
        frame.classList.add('missing');
        frame.disabled = true;
      }
    };
    if (imageCache.has(key)) {
      show(imageCache.get(key));
    } else {
      loadImage(target, image.id, key).then(show);
    }
    const source = { target, imageId: image.id };
    frame.addEventListener('click', () => {
      if (picture.src) {
        openLightbox(picture.src, source);
      }
    });
    frame.addEventListener('contextmenu', (event) => {
      if (picture.src) {
        openImageMenu(event, source);
      }
    });
    return frame;
  }

  function embedNode(embed) {
    const card = document.createElement('div');
    card.className = 'message-embed';
    const href = linkTarget(String(embed.url ?? ''));
    if (embed.siteName) {
      const site = document.createElement('span');
      site.className = 'embed-site';
      site.textContent = embed.siteName;
      card.append(site);
    }
    if (embed.title) {
      const title = href ? linkNode(embed.title, href) : document.createElement('span');
      title.classList.add('embed-title');
      title.textContent = embed.title;
      card.append(title);
    }
    if (embed.description) {
      const description = document.createElement('p');
      description.className = 'embed-description';
      description.textContent = embed.description;
      card.append(description);
    }
    if (embed.image && embed.image.id) {
      card.append(imageNode(embed.image, EMBED_BOUNDS, 'message-image embed-image'));
    }
    return card;
  }

  /* cartao de arquivo */

  function transferKey(key, fileId) {
    return `${key}/${fileId}`;
  }

  function paintFile(entry) {
    const {
      file, target, status, bar, action,
    } = entry;
    const key = transferKey(target.key, file.id);
    const moving = progress.get(key);
    const size = formatBytes(Number(file.size) || 0);
    let text = size;
    let fraction = null;
    let busy = false;

    if (file.pending) {
      const fetching = fetches.get(key);
      if (moving && moving.direction === 'down' && moving.total > 0) {
        fraction = moving.done / moving.total;
        text = `Recebendo · ${formatPercent(fraction)} · ${formatBytes(moving.done)} de ${size}`;
        busy = true;
      } else if (fetching && fetching.status === 'failed') {
        text = `${size} · não chegou. Quem mandou precisa estar com o Telinha aberto.`;
      } else if (fetching) {
        text = fetching.status === 'receiving' ? `Recebendo · ${size}` : `${size} · procurando quem tem o arquivo…`;
        busy = true;
      } else {
        text = `${size} · ainda não chegou`;
      }
    } else if (moving && moving.direction === 'up' && moving.total > 0) {
      fraction = moving.done / moving.total;
      const who = moving.peers > 1 ? ` para ${moving.peers} pessoas` : '';
      text = `Enviando${who} · ${formatPercent(fraction)} de ${size}`;
    }

    status.textContent = text;
    setBar(bar, fraction);
    action.hidden = busy;
    action.title = file.pending ? 'Baixar de novo' : 'Salvar arquivo';
    action.setAttribute('aria-label', file.pending ? `Baixar ${file.name}` : `Salvar ${file.name}`);
  }

  async function fileAction(entry) {
    const { file, target, action } = entry;
    const key = transferKey(target.key, file.id);
    action.disabled = true;
    try {
      if (file.pending) {
        fetches.set(key, { status: 'waiting' });
        paintFile(entry);
        if (!await window.telinha.downloadChatFile(target.spaceId, target.channelId, file.id)) {
          fetches.delete(key);
          paintFile(entry);
        }
      } else if (await window.telinha.saveChatFile(target.spaceId, target.channelId, file.id)) {
        nodes.hint.textContent = `Arquivo salvo: ${file.name}`;
      }
    } catch (error) {
      nodes.hint.textContent = `Não consegui salvar: ${cleanError(error)}`;
    } finally {
      action.disabled = false;
    }
  }

  function fileNode(file) {
    const target = conversation;
    const card = document.createElement('div');
    card.className = 'message-file';
    const icon = document.createElement('span');
    icon.className = 'file-icon';
    icon.innerHTML = FILE_ICON;

    const info = document.createElement('div');
    info.className = 'file-info';
    const name = document.createElement('span');
    name.className = 'file-name';
    name.textContent = file.name;
    name.title = file.name;
    const status = document.createElement('span');
    status.className = 'file-size';
    const bar = document.createElement('span');
    bar.className = 'progress-bar';
    bar.append(document.createElement('span'));
    info.append(name, status, bar);

    const action = document.createElement('button');
    action.type = 'button';
    action.className = 'icon-button file-download';
    action.innerHTML = DOWNLOAD_ICON;

    const entry = {
      file, target, status, bar, action,
    };
    action.addEventListener('click', () => fileAction(entry));
    card.append(icon, info, action);
    fileCards.set(transferKey(target.key, file.id), entry);
    paintFile(entry);
    return card;
  }

  /* anexos enviados */

  function canvasBlob(canvas, type, quality) {
    return new Promise((resolve) => canvas.toBlob(resolve, type, quality));
  }

  async function encodeImage(file, sides = IMAGE_SIDES, maxBytes = MAX_IMAGE_BYTES) {
    const bitmap = await createImageBitmap(file);
    try {
      for (const side of sides) {
        const scale = Math.min(1, side / Math.max(bitmap.width, bitmap.height));
        const width = Math.max(1, Math.round(bitmap.width * scale));
        const height = Math.max(1, Math.round(bitmap.height * scale));
        const canvas = document.createElement('canvas');
        canvas.width = width;
        canvas.height = height;
        canvas.getContext('2d').drawImage(bitmap, 0, 0, width, height);
        for (const quality of IMAGE_QUALITIES) {
          const blob = await canvasBlob(canvas, 'image/webp', quality);
          if (blob && blob.size <= maxBytes) {
            return { blob, width, height, mime: blob.type || 'image/webp' };
          }
        }
      }
    } finally {
      bitmap.close();
    }
    throw new Error('imagem grande demais');
  }

  async function blobToBase64(blob) {
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = '';
    for (let offset = 0; offset < bytes.length; offset += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
    }
    return btoa(binary);
  }

  async function imageAttachment(file) {
    if (file.type === 'image/gif' && file.size <= MAX_IMAGE_BYTES) {
      const bitmap = await createImageBitmap(file);
      const { width, height } = bitmap;
      bitmap.close();
      return {
        kind: 'image', data: await blobToBase64(file), mime: 'image/gif', width, height, name: file.name, size: file.size,
      };
    }
    const encoded = await encodeImage(file);
    return {
      kind: 'image',
      data: await blobToBase64(encoded.blob),
      mime: encoded.mime,
      width: encoded.width,
      height: encoded.height,
      name: file.name || 'Imagem colada',
      size: encoded.blob.size,
    };
  }

  function showPending() {
    nodes.pending.hidden = false;
    setBar(nodes.pendingBar, null);
    if (!attachment) {
      nodes.pendingImage.hidden = true;
      nodes.pendingIcon.hidden = true;
      nodes.pendingName.textContent = 'Preparando o anexo…';
      return;
    }
    const image = attachment.kind === 'image';
    nodes.pendingImage.hidden = !image;
    nodes.pendingIcon.hidden = image;
    if (image) {
      nodes.pendingImage.src = `data:${attachment.mime};base64,${attachment.data}`;
    } else {
      nodes.pendingImage.removeAttribute('src');
    }
    const details = image
      ? `${attachment.width}×${attachment.height} · ${formatBytes(attachment.size)}`
      : formatBytes(attachment.size);
    nodes.pendingName.textContent = `${attachment.name} · ${details}`;
  }

  function showUpload(done, total) {
    if (!attachment || nodes.pending.hidden) {
      return;
    }
    const fraction = total > 0 ? done / total : 0;
    nodes.pendingName.textContent = `Preparando ${attachment.name} · ${formatPercent(fraction)}`;
    setBar(nodes.pendingBar, fraction);
  }

  function clearAttachment() {
    attachment = null;
    nodes.pending.hidden = true;
    nodes.pendingImage.removeAttribute('src');
    nodes.pendingName.textContent = '';
    setBar(nodes.pendingBar, null);
    nodes.file.value = '';
  }

  async function attachFile(file) {
    if (!conversation || !file || sending) {
      return;
    }
    preparing = true;
    attachment = null;
    showPending();
    try {
      const imageLike = file.type.startsWith('image/') && file.type !== 'image/svg+xml';
      if (imageLike) {
        try {
          attachment = await imageAttachment(file);
        } catch {
          attachment = null;
        }
      }
      if (!attachment) {
        if (file.size === 0 || file.size > MAX_FILE_BYTES) {
          clearAttachment();
          nodes.hint.textContent = 'O arquivo precisa ter no máximo 1 GB.';
          return;
        }
        attachment = {
          kind: 'file',
          source: file,
          mime: file.type || 'application/octet-stream',
          name: file.name || 'arquivo',
          size: file.size,
        };
      }
      showPending();
      nodes.input.focus();
    } catch {
      clearAttachment();
      nodes.hint.textContent = 'Não consegui ler esse arquivo.';
    } finally {
      preparing = false;
    }
  }

  /* previa de link na caixa de texto */

  function hostOf(url) {
    try {
      return new URL(url).hostname.replace(/^www\./i, '');
    } catch {
      return '';
    }
  }

  async function embedImage(source) {
    const blob = new Blob([source.data], { type: source.mime });
    const encoded = await encodeImage(blob, EMBED_IMAGE_SIDES, EMBED_IMAGE_BYTES);
    return {
      data: await blobToBase64(encoded.blob), mime: encoded.mime, width: encoded.width, height: encoded.height,
    };
  }

  // A previa e montada aqui, por quem manda: quem recebe nao abre o link de ninguem, e a pagina
  // nao fica sabendo quem leu a mensagem.
  async function buildEmbed(url) {
    const preview = await window.telinha.linkPreview(url).catch(() => null);
    if (!preview) {
      return null;
    }
    const image = preview.image && preview.image.data ? await embedImage(preview.image).catch(() => null) : null;
    const data = {
      url,
      title: String(preview.title || ''),
      description: String(preview.description || ''),
      siteName: String(preview.siteName || ''),
      image,
    };
    return data.title || data.description || data.image ? data : null;
  }

  function showEmbed() {
    const current = draftEmbed;
    const visible = Boolean(current && current.status !== 'none');
    nodes.embed.hidden = !visible;
    nodes.embed.classList.toggle('loading', Boolean(current && current.status === 'loading'));
    if (!visible || current.status === 'loading' || !current.data.image) {
      nodes.embedImage.hidden = true;
      nodes.embedImage.removeAttribute('src');
    }
    if (!visible) {
      return;
    }
    if (current.status === 'loading') {
      nodes.embedSite.textContent = hostOf(current.url);
      nodes.embedTitle.textContent = 'Carregando a prévia do link…';
      nodes.embedText.textContent = '';
      return;
    }
    const { data } = current;
    nodes.embedSite.textContent = data.siteName || hostOf(data.url);
    nodes.embedTitle.textContent = data.title || data.url;
    nodes.embedText.textContent = data.description;
    if (data.image) {
      nodes.embedImage.src = `data:${data.image.mime};base64,${data.image.data}`;
      nodes.embedImage.hidden = false;
    }
  }

  function clearEmbed() {
    clearTimeout(embedTimer);
    embedTimer = null;
    draftEmbed = null;
    showEmbed();
  }

  function refreshEmbed() {
    clearTimeout(embedTimer);
    embedTimer = null;
    const url = conversation ? firstLink(nodes.input.value) : null;
    if (!url || dismissedLinks.has(url)) {
      if (draftEmbed) {
        clearEmbed();
      }
      return;
    }
    if (draftEmbed && draftEmbed.url === url) {
      return;
    }
    const current = {
      url, status: 'loading', data: null, promise: null,
    };
    current.promise = buildEmbed(url).then((data) => {
      current.data = data;
      current.status = data ? 'ready' : 'none';
      if (draftEmbed === current) {
        showEmbed();
      }
      return data;
    });
    draftEmbed = current;
    showEmbed();
  }

  function scheduleEmbed() {
    clearTimeout(embedTimer);
    embedTimer = setTimeout(refreshEmbed, EMBED_DELAY_MS);
  }

  async function embedFor(text) {
    if (embedTimer) {
      refreshEmbed();
    }
    const current = draftEmbed;
    if (!current || firstLink(text) !== current.url) {
      return null;
    }
    if (current.status === 'loading') {
      await Promise.race([current.promise, delay(EMBED_WAIT_MS)]);
    }
    return current.status === 'ready' ? current.data : null;
  }

  /* mensagens */

  function groupItem(first) {
    const item = document.createElement('li');
    item.className = first.mine ? 'message-group mine' : 'message-group';
    const author = authorOf(first);

    const avatar = document.createElement('span');
    avatar.className = 'avatar';
    avatar.textContent = initial(author);
    avatar.style.background = Call.avatarColor(author);
    const avatarUrl = conversation && conversation.avatarFor ? conversation.avatarFor(first) : null;
    if (avatarUrl) {
      avatar.textContent = '';
      avatar.classList.add('has-image');
      const image = document.createElement('img');
      image.src = avatarUrl;
      image.alt = '';
      avatar.append(image);
    }

    const body = document.createElement('div');
    const head = document.createElement('div');
    head.className = 'message-head';
    const name = document.createElement('span');
    name.className = 'message-author';
    name.textContent = author;
    const time = document.createElement('span');
    time.className = 'message-time';
    time.textContent = formatTime(first.sentAt);
    head.append(name, time);
    body.append(head);
    item.append(avatar, body);
    return { item, body };
  }

  function messageLine(message) {
    const line = document.createElement('div');
    line.className = 'message-line';
    if (message.text) {
      const text = document.createElement('p');
      text.className = 'message-text';
      appendLinked(text, message.text);
      text.title = formatTime(message.sentAt);
      line.append(text);
    }
    if (message.embed && (message.embed.title || message.embed.description || message.embed.image)) {
      line.append(embedNode(message.embed));
    }
    if (message.image && message.image.id) {
      line.append(imageNode(message.image, IMAGE_BOUNDS));
    }
    if (message.file && message.file.id) {
      line.append(fileNode(message.file));
    }
    if (message.mine && message.delivered === false) {
      const failed = document.createElement('span');
      failed.className = 'message-failed';
      failed.textContent = '(não enviada, sem conexão com o servidor)';
      line.append(failed);
    }

    const invite = message.text ? message.text.match(INVITE_PATTERN) : null;
    if (invite && conversation && conversation.onInvite) {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'button primary small invite-button';
      button.textContent = 'Entrar no servidor';
      button.addEventListener('click', () => conversation.onInvite(invite[0]));
      line.append(button);
    }
    return line;
  }

  function render() {
    fileCards.clear();
    const groups = [];
    let current = null;
    let previous = null;
    for (const message of messages) {
      const sameGroup = previous && previous.mine === message.mine
        && (message.mine || (previous.memberId ?? previous.author) === (message.memberId ?? message.author))
        && message.sentAt - previous.sentAt < GROUP_WINDOW_MS;
      if (!sameGroup) {
        current = groupItem(message);
        groups.push(current.item);
      }
      current.body.append(messageLine(message));
      previous = message;
    }
    nodes.messages.replaceChildren(...groups);
  }

  function renderHint() {
    nodes.hint.textContent = conversation && conversation.hint ? conversation.hint() : '';
  }

  function applyChrome() {
    nodes.introAvatar.textContent = conversation.introIcon ?? initial(conversation.title);
    nodes.introTitle.textContent = conversation.introTitle ?? conversation.title;
    nodes.introText.textContent = conversation.introText ?? '';
    nodes.input.placeholder = conversation.placeholder ?? 'Mensagem';
    renderHint();
  }

  async function open(next) {
    const changed = !conversation || conversation.key !== next.key;
    conversation = next;
    applyChrome();
    if (!changed) {
      render();
      return;
    }

    nodes.input.value = '';
    clearAttachment();
    clearEmbed();
    dismissedLinks.clear();
    messages = [];
    render();
    const token = ++loadToken;
    const history = await window.telinha.chatHistory(next.spaceId, next.channelId).catch(() => []);
    if (token !== loadToken) {
      return;
    }
    messages = Array.isArray(history) ? [...history] : [];
    render();
    scrollToBottom();
    nodes.input.focus();
  }

  function refresh(next) {
    if (conversation && next && next.key === conversation.key) {
      conversation = next;
      applyChrome();
      render();
    } else {
      renderHint();
    }
  }

  function close() {
    conversation = null;
    loadToken += 1;
    clearAttachment();
    clearEmbed();
  }

  function receive(key, message) {
    if (!conversation || conversation.key !== key) {
      return false;
    }
    if (messages.some((existing) => existing.id === message.id)) {
      return true;
    }
    const stick = nearBottom();
    messages.push(message);
    messages.sort((a, b) => a.sentAt - b.sentAt);
    render();
    if (stick || message.mine) {
      scrollToBottom();
    }
    return true;
  }

  // Uma mensagem guardada mudou: o arquivo chegou, ou o envio pelo servidor terminou.
  function update(key, message) {
    if (message.file && message.file.id) {
      const id = transferKey(key, message.file.id);
      if (!message.file.pending) {
        fetches.delete(id);
      }
      if (message.mine && !message.file.offerId && message.delivered !== undefined) {
        progress.delete(id);
      }
    }
    if (!conversation || conversation.key !== key) {
      return false;
    }
    const index = messages.findIndex((existing) => existing.id === message.id);
    if (index < 0) {
      return false;
    }
    const stick = nearBottom();
    messages[index] = message;
    render();
    if (stick) {
      scrollToBottom();
    }
    return true;
  }

  function transferProgress(info) {
    const id = transferKey(info.key, info.fileId);
    if (info.peers > 0) {
      progress.set(id, info);
    } else {
      progress.delete(id);
    }
    const entry = fileCards.get(id);
    if (entry) {
      paintFile(entry);
    }
  }

  function fetchStatus(info) {
    const id = transferKey(info.key, info.fileId);
    fetches.set(id, { status: info.status });
    const entry = fileCards.get(id);
    if (entry) {
      paintFile(entry);
    }
  }

  function uploadProgress(info) {
    if (info && info.uploadId === uploadId) {
      showUpload(info.done, info.total);
    }
  }

  function currentKey() {
    return conversation ? conversation.key : null;
  }

  async function submit() {
    const text = nodes.input.value.trim();
    if (!conversation || (!text && !attachment) || sending || preparing) {
      return;
    }
    sending = true;
    nodes.pendingRemove.disabled = true;
    const target = conversation;
    const current = attachment;
    const image = current && current.kind === 'image'
      ? {
        data: current.data, mime: current.mime, width: current.width, height: current.height,
      }
      : null;
    const file = current && current.kind === 'file'
      ? { source: current.source, name: current.name, mime: current.mime }
      : null;
    uploadId = file ? crypto.randomUUID() : null;
    if (file) {
      nodes.hint.textContent = `Enviando ${file.name}…`;
    }
    try {
      const embed = text ? await embedFor(text) : null;
      const message = await window.telinha.sendChat({
        spaceId: target.spaceId,
        channelId: target.channelId,
        text,
        image,
        file,
        embed,
        uploadId,
      });
      if (conversation === target) {
        nodes.input.value = '';
        clearAttachment();
        clearEmbed();
        dismissedLinks.clear();
      }
      renderHint();
      if (message) {
        if (message.image && image) {
          cacheImage(`${target.key}/${message.image.id}`, `data:${image.mime};base64,${image.data}`);
        }
        if (message.embed && message.embed.image && embed && embed.image) {
          cacheImage(`${target.key}/${message.embed.image.id}`, `data:${embed.image.mime};base64,${embed.image.data}`);
        }
        receive(target.key, message);
      }
    } catch (error) {
      nodes.hint.textContent = `Não consegui enviar: ${cleanError(error)}`;
      if (attachment === current && current) {
        showPending();
      }
    } finally {
      sending = false;
      uploadId = null;
      nodes.pendingRemove.disabled = false;
    }
  }

  function linkFrom(event) {
    const link = event.target instanceof Element ? event.target.closest('a[href]') : null;
    return link && nodes.messages.contains(link) ? link.href : null;
  }

  function bind(options) {
    profileName = options.profileName;
    nodes.input.addEventListener('input', scheduleEmbed);
    nodes.input.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
        event.preventDefault();
        submit();
      }
    });
    nodes.input.addEventListener('paste', (event) => {
      const files = event.clipboardData ? [...event.clipboardData.files] : [];
      if (files.length > 0) {
        event.preventDefault();
        attachFile(files[0]);
      }
    });
    nodes.form.addEventListener('submit', (event) => {
      event.preventDefault();
      submit();
    });

    nodes.attach.addEventListener('click', () => nodes.file.click());
    nodes.file.addEventListener('change', () => attachFile(nodes.file.files[0]));
    nodes.pendingRemove.addEventListener('click', clearAttachment);
    nodes.embedRemove.addEventListener('click', () => {
      if (draftEmbed) {
        dismissedLinks.add(draftEmbed.url);
      }
      clearEmbed();
      nodes.input.focus();
    });

    nodes.messages.addEventListener('click', (event) => {
      const href = linkFrom(event);
      if (href) {
        event.preventDefault();
        openLink(href);
      }
    });
    nodes.messages.addEventListener('auxclick', (event) => {
      const href = event.button === 1 ? linkFrom(event) : null;
      if (href) {
        event.preventDefault();
        openLink(href);
      }
    });
    nodes.messages.addEventListener('contextmenu', (event) => {
      const href = linkFrom(event);
      if (href) {
        openLinkMenu(event, href);
      }
    });

    nodes.view.addEventListener('dragover', (event) => {
      if (event.dataTransfer && [...event.dataTransfer.types].includes('Files')) {
        event.preventDefault();
        nodes.view.classList.add('dropping');
      }
    });
    nodes.view.addEventListener('dragleave', (event) => {
      if (!nodes.view.contains(event.relatedTarget)) {
        nodes.view.classList.remove('dropping');
      }
    });
    nodes.view.addEventListener('drop', (event) => {
      event.preventDefault();
      nodes.view.classList.remove('dropping');
      const files = event.dataTransfer ? [...event.dataTransfer.files] : [];
      if (files.length > 0) {
        attachFile(files[0]);
      }
    });

    nodes.lightbox.addEventListener('click', closeLightbox);
    nodes.lightbox.addEventListener('contextmenu', (event) => openImageMenu(event, lightboxSource));
    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape' && !nodes.lightbox.hidden) {
        closeLightbox();
      }
    });
  }

  return {
    bind, close, currentKey, fetchStatus, open, receive, refresh, transferProgress, update, uploadProgress,
  };
})();
