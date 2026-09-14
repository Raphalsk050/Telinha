'use strict';

const Chat = (() => {
  const GROUP_WINDOW_MS = 5 * 60 * 1000;
  const INVITE_PATTERN = /TELINHA-GRUPO\.[A-Za-z0-9_-]+/;
  const MAX_IMAGE_BYTES = 380 * 1024;
  const MAX_FILE_BYTES = 8 * 1024 * 1024;
  const IMAGE_SIDES = [1920, 1600, 1280, 960, 720];
  const IMAGE_QUALITIES = [0.85, 0.72, 0.6];
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
    pendingRemove: find('chat-pending-remove'),
    hint: find('chat-hint'),
    lightbox: find('lightbox'),
    lightboxImage: find('lightbox-image'),
  };

  const imageCache = new Map();
  const imageLoads = new Map();

  let conversation = null;
  let messages = [];
  let loadToken = 0;
  let sending = false;
  let preparing = false;
  let attachment = null;
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
    return `${(bytes / (1024 * 1024)).toLocaleString('pt-BR', { maximumFractionDigits: 1 })} MB`;
  }

  function cleanError(error) {
    return String(error && error.message ? error.message : error)
      .replace(/^Error invoking remote method '[^']+': (Error: )?/, '');
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

  function openLightbox(url) {
    nodes.lightboxImage.src = url;
    nodes.lightbox.hidden = false;
  }

  function closeLightbox() {
    nodes.lightbox.hidden = true;
    nodes.lightboxImage.removeAttribute('src');
  }

  function imageNode(image) {
    const frame = document.createElement('button');
    frame.type = 'button';
    frame.className = 'message-image';
    frame.title = 'Abrir imagem';
    const width = Math.max(1, Number(image.width) || 1);
    const height = Math.max(1, Number(image.height) || 1);
    const scale = Math.min(1, 400 / width, 300 / height);
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
    frame.addEventListener('click', () => {
      if (picture.src) {
        openLightbox(picture.src);
      }
    });
    return frame;
  }

  function fileNode(file) {
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
    const size = document.createElement('span');
    size.className = 'file-size';
    size.textContent = formatBytes(Number(file.size) || 0);
    info.append(name, size);

    const download = document.createElement('button');
    download.type = 'button';
    download.className = 'icon-button file-download';
    download.title = 'Salvar arquivo';
    download.setAttribute('aria-label', `Salvar ${file.name}`);
    download.innerHTML = DOWNLOAD_ICON;
    const target = conversation;
    download.addEventListener('click', async () => {
      download.disabled = true;
      try {
        if (await window.telinha.saveChatFile(target.spaceId, target.channelId, file.id)) {
          nodes.hint.textContent = `Arquivo salvo: ${file.name}`;
        }
      } catch (error) {
        nodes.hint.textContent = `Não consegui salvar: ${cleanError(error)}`;
      } finally {
        download.disabled = false;
      }
    });

    card.append(icon, info, download);
    return card;
  }

  /* anexos enviados */

  function canvasBlob(canvas, type, quality) {
    return new Promise((resolve) => canvas.toBlob(resolve, type, quality));
  }

  async function encodeImage(file) {
    const bitmap = await createImageBitmap(file);
    try {
      for (const side of IMAGE_SIDES) {
        const scale = Math.min(1, side / Math.max(bitmap.width, bitmap.height));
        const width = Math.max(1, Math.round(bitmap.width * scale));
        const height = Math.max(1, Math.round(bitmap.height * scale));
        const canvas = document.createElement('canvas');
        canvas.width = width;
        canvas.height = height;
        canvas.getContext('2d').drawImage(bitmap, 0, 0, width, height);
        for (const quality of IMAGE_QUALITIES) {
          const blob = await canvasBlob(canvas, 'image/webp', quality);
          if (blob && blob.size <= MAX_IMAGE_BYTES) {
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

  function clearAttachment() {
    attachment = null;
    nodes.pending.hidden = true;
    nodes.pendingImage.removeAttribute('src');
    nodes.pendingName.textContent = '';
    nodes.file.value = '';
  }

  async function attachFile(file) {
    if (!conversation || !file) {
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
          nodes.hint.textContent = 'O arquivo precisa ter no máximo 8 MB.';
          return;
        }
        attachment = {
          kind: 'file',
          data: await blobToBase64(file),
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
      text.textContent = message.text;
      text.title = formatTime(message.sentAt);
      line.append(text);
    }
    if (message.image && message.image.id) {
      line.append(imageNode(message.image));
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

  function autoGrow() {
    nodes.input.style.height = 'auto';
    nodes.input.style.height = `${Math.min(nodes.input.scrollHeight, 200)}px`;
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
    autoGrow();
    clearAttachment();
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

  function currentKey() {
    return conversation ? conversation.key : null;
  }

  async function submit() {
    const text = nodes.input.value.trim();
    if (!conversation || (!text && !attachment) || sending || preparing) {
      return;
    }
    sending = true;
    const target = conversation;
    const current = attachment;
    const image = current && current.kind === 'image'
      ? { data: current.data, mime: current.mime, width: current.width, height: current.height }
      : null;
    const file = current && current.kind === 'file'
      ? { data: current.data, mime: current.mime, name: current.name }
      : null;
    if (file) {
      nodes.hint.textContent = `Enviando ${file.name}…`;
    }
    try {
      const message = await window.telinha.sendChat(target.spaceId, target.channelId, text, image, file);
      nodes.input.value = '';
      autoGrow();
      clearAttachment();
      renderHint();
      if (message) {
        if (message.image && image) {
          cacheImage(`${target.key}/${message.image.id}`, `data:${image.mime};base64,${image.data}`);
        }
        receive(target.key, message);
      }
    } catch (error) {
      nodes.hint.textContent = `Não consegui enviar: ${cleanError(error)}`;
    } finally {
      sending = false;
    }
  }

  function bind(options) {
    profileName = options.profileName;
    nodes.input.addEventListener('input', autoGrow);
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
    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape' && !nodes.lightbox.hidden) {
        closeLightbox();
      }
    });
  }

  return { bind, close, currentKey, open, receive, refresh };
})();
