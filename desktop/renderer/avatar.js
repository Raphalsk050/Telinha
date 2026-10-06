'use strict';

const AvatarEditor = (() => {
  const MAX_ZOOM = 4;
  const MAX_BYTES = 180 * 1024;
  // Cada tipo de imagem tem a sua moldura de corte. A pessoa arrasta e aproxima a imagem por tras
  // dela, e o que sai tem sempre o tamanho de "output", seja qual for a imagem escolhida.
  const FRAMES = {
    avatar: {
      view: [300, 300], crop: [240, 240], output: [256, 256], round: true, title: 'Editar imagem',
    },
    banner: {
      view: [480, 220], crop: [440, 154], output: [600, 210], round: false, title: 'Recortar banner',
    },
  };
  const CROP_CORNER = 8;
  const find = (id) => document.getElementById(id);

  const nodes = {
    picker: find('avatar-picker'),
    pickerClose: find('avatar-picker-close'),
    pickerStatus: find('avatar-picker-status'),
    upload: find('avatar-upload'),
    file: find('avatar-file'),
    recentsTitle: find('avatar-recents-title'),
    recents: find('avatar-recents'),
    remove: find('avatar-remove'),
    editor: find('avatar-editor'),
    editorTitle: find('avatar-editor-title'),
    editorClose: find('avatar-editor-close'),
    canvas: find('avatar-canvas'),
    zoom: find('avatar-zoom'),
    rotate: find('avatar-rotate'),
    reset: find('avatar-reset'),
    cancel: find('avatar-cancel'),
    apply: find('avatar-apply'),
    status: find('avatar-status'),
  };

  const view = {
    bitmap: null, zoom: 1, rotation: 0, panX: 0, panY: 0, drag: null,
  };
  let frame = FRAMES.avatar;
  let options = null;

  function isOpen() {
    return !nodes.picker.hidden || !nodes.editor.hidden;
  }

  function releaseBitmap() {
    if (view.bitmap) {
      view.bitmap.close();
      view.bitmap = null;
    }
  }

  function closeAll() {
    nodes.picker.hidden = true;
    nodes.editor.hidden = true;
    view.drag = null;
    releaseBitmap();
  }

  function useFrame(next) {
    frame = next;
    nodes.editorTitle.textContent = frame.title;
    // So a largura e fixa: numa janela estreita a tela do corte encolhe sem deformar.
    nodes.canvas.style.width = `${frame.view[0]}px`;
    nodes.canvas.style.height = 'auto';
    nodes.canvas.style.aspectRatio = `${frame.view[0]} / ${frame.view[1]}`;
    nodes.editor.querySelector('.dialog').classList.toggle('crop-wide', frame.view[0] > 300);
  }

  function openPicker(next) {
    options = next;
    useFrame(FRAMES.avatar);
    nodes.pickerStatus.textContent = '';
    nodes.recents.replaceChildren(...next.recents.map((recent) => {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'avatar-recent';
      button.title = 'Usar este avatar';
      button.setAttribute('aria-label', 'Usar este avatar');
      const image = document.createElement('img');
      image.src = recent.url;
      image.alt = '';
      button.append(image);
      button.addEventListener('click', () => {
        closeAll();
        next.onRecent(recent.hash);
      });
      return button;
    }));
    nodes.recentsTitle.hidden = next.recents.length === 0;
    nodes.remove.hidden = !next.hasAvatar;
    nodes.editor.hidden = true;
    nodes.picker.hidden = false;
  }

  function metrics() {
    const rotated = view.rotation % 180 !== 0;
    const width = rotated ? view.bitmap.height : view.bitmap.width;
    const height = rotated ? view.bitmap.width : view.bitmap.height;
    const scale = Math.max(frame.crop[0] / width, frame.crop[1] / height) * view.zoom;
    return { width: width * scale, height: height * scale, scale };
  }

  // A imagem nunca deixa um pedaco da moldura vazio.
  function clampPan() {
    const { width, height } = metrics();
    const maxX = Math.max(0, (width - frame.crop[0]) / 2);
    const maxY = Math.max(0, (height - frame.crop[1]) / 2);
    view.panX = Math.max(-maxX, Math.min(maxX, view.panX));
    view.panY = Math.max(-maxY, Math.min(maxY, view.panY));
  }

  function paint(context, width, height, factor) {
    const { scale } = metrics();
    context.save();
    context.translate(width / 2 + view.panX * factor, height / 2 + view.panY * factor);
    context.rotate((view.rotation * Math.PI) / 180);
    context.scale(scale * factor, scale * factor);
    context.drawImage(view.bitmap, -view.bitmap.width / 2, -view.bitmap.height / 2);
    context.restore();
  }

  function traceCrop(context, counterclockwise) {
    const [viewWidth, viewHeight] = frame.view;
    const [cropWidth, cropHeight] = frame.crop;
    if (frame.round) {
      context.arc(viewWidth / 2, viewHeight / 2, cropWidth / 2, 0, Math.PI * 2, counterclockwise);
    } else {
      context.roundRect((viewWidth - cropWidth) / 2, (viewHeight - cropHeight) / 2, cropWidth, cropHeight, CROP_CORNER);
    }
  }

  function draw() {
    if (!view.bitmap) {
      return;
    }
    const [viewWidth, viewHeight] = frame.view;
    const ratio = window.devicePixelRatio || 1;
    const { canvas } = nodes;
    canvas.width = Math.round(viewWidth * ratio);
    canvas.height = Math.round(viewHeight * ratio);
    const context = canvas.getContext('2d');
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.fillStyle = '#1e1f22';
    context.fillRect(0, 0, viewWidth, viewHeight);
    paint(context, viewWidth, viewHeight, 1);

    context.fillStyle = 'rgba(0, 0, 0, 0.55)';
    context.beginPath();
    context.rect(0, 0, viewWidth, viewHeight);
    traceCrop(context, true);
    context.fill('evenodd');

    context.strokeStyle = '#ffffff';
    context.lineWidth = 3;
    context.beginPath();
    traceCrop(context, false);
    context.stroke();
  }

  function resetView() {
    view.zoom = 1;
    view.rotation = 0;
    view.panX = 0;
    view.panY = 0;
    nodes.zoom.value = '1';
    draw();
  }

  // Devolve o motivo quando a imagem nao pode ser aberta.
  async function edit(file) {
    if (!file.type.startsWith('image/')) {
      return 'Escolha um arquivo de imagem.';
    }
    try {
      const bitmap = await createImageBitmap(file);
      releaseBitmap();
      view.bitmap = bitmap;
      nodes.status.textContent = '';
      nodes.picker.hidden = true;
      nodes.editor.hidden = false;
      resetView();
      return '';
    } catch {
      return 'Não consegui abrir essa imagem. Tente PNG, JPG, WebP ou GIF.';
    }
  }

  async function loadFile(file) {
    nodes.file.value = '';
    if (file) {
      nodes.pickerStatus.textContent = await edit(file);
    }
  }

  // Abre direto no corte, para uma imagem que a pessoa ja escolheu em outro lugar.
  async function openCrop(file, next) {
    options = next;
    useFrame(FRAMES[next.frame]);
    const problem = await edit(file);
    if (problem && next.onError) {
      next.onError(problem);
    }
  }

  function canvasBlob(canvas, type, quality) {
    return new Promise((resolve) => canvas.toBlob(resolve, type, quality));
  }

  async function blobToBase64(blob) {
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = '';
    for (let offset = 0; offset < bytes.length; offset += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
    }
    return btoa(binary);
  }

  async function apply() {
    if (!view.bitmap || !options) {
      return;
    }
    nodes.apply.disabled = true;
    try {
      const [outputWidth, outputHeight] = frame.output;
      const canvas = document.createElement('canvas');
      canvas.width = outputWidth;
      canvas.height = outputHeight;
      const context = canvas.getContext('2d');
      context.imageSmoothingQuality = 'high';
      paint(context, outputWidth, outputHeight, outputWidth / frame.crop[0]);

      let blob = null;
      for (const quality of [0.9, 0.8, 0.65, 0.5]) {
        blob = await canvasBlob(canvas, 'image/webp', quality);
        if (blob && blob.size <= MAX_BYTES) {
          break;
        }
      }
      if (!blob || blob.size > MAX_BYTES) {
        nodes.status.textContent = 'A imagem ficou grande demais, tente outra.';
        return;
      }
      const data = await blobToBase64(blob);
      const { onApply } = options;
      closeAll();
      await onApply(blob.type || 'image/webp', data);
    } finally {
      nodes.apply.disabled = false;
    }
  }

  nodes.upload.addEventListener('click', () => nodes.file.click());
  nodes.file.addEventListener('change', () => loadFile(nodes.file.files[0]));
  nodes.pickerClose.addEventListener('click', closeAll);
  nodes.editorClose.addEventListener('click', closeAll);
  nodes.cancel.addEventListener('click', closeAll);
  nodes.reset.addEventListener('click', resetView);
  nodes.apply.addEventListener('click', apply);
  nodes.remove.addEventListener('click', () => {
    const current = options;
    closeAll();
    if (current) {
      current.onRemove();
    }
  });
  nodes.rotate.addEventListener('click', () => {
    view.rotation = (view.rotation + 90) % 360;
    clampPan();
    draw();
  });
  nodes.zoom.addEventListener('input', () => {
    view.zoom = Number(nodes.zoom.value);
    clampPan();
    draw();
  });

  nodes.canvas.addEventListener('pointerdown', (event) => {
    if (!view.bitmap) {
      return;
    }
    nodes.canvas.setPointerCapture(event.pointerId);
    view.drag = { x: event.clientX, y: event.clientY, panX: view.panX, panY: view.panY };
  });
  nodes.canvas.addEventListener('pointermove', (event) => {
    if (!view.drag) {
      return;
    }
    const factor = frame.view[0] / nodes.canvas.getBoundingClientRect().width;
    view.panX = view.drag.panX + (event.clientX - view.drag.x) * factor;
    view.panY = view.drag.panY + (event.clientY - view.drag.y) * factor;
    clampPan();
    draw();
  });
  const endDrag = () => {
    view.drag = null;
  };
  nodes.canvas.addEventListener('pointerup', endDrag);
  nodes.canvas.addEventListener('pointercancel', endDrag);
  nodes.canvas.addEventListener('wheel', (event) => {
    if (!view.bitmap) {
      return;
    }
    event.preventDefault();
    view.zoom = Math.max(1, Math.min(MAX_ZOOM, view.zoom * (event.deltaY < 0 ? 1.08 : 1 / 1.08)));
    nodes.zoom.value = String(view.zoom);
    clampPan();
    draw();
  }, { passive: false });

  nodes.picker.addEventListener('dragover', (event) => event.preventDefault());
  nodes.picker.addEventListener('drop', (event) => {
    event.preventDefault();
    const files = event.dataTransfer ? [...event.dataTransfer.files] : [];
    loadFile(files.find((item) => item.type.startsWith('image/')) || files[0]);
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && isOpen()) {
      closeAll();
    }
  });

  return { isOpen, openPicker, openCrop };
})();
