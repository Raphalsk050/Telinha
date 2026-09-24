'use strict';

const dns = require('node:dns').promises;
const net = require('node:net');

const TIMEOUT_MS = 8000;
const MAX_HTML_BYTES = 1024 * 1024;
const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
const MAX_REDIRECTS = 5;
const MAX_URL_LENGTH = 2048;
const CACHE_LIMIT = 60;
const CACHE_TTL_MS = 10 * 60 * 1000;
const MAX_TITLE = 200;
const MAX_DESCRIPTION = 400;
const MAX_SITE = 80;
const USER_AGENT = 'Mozilla/5.0 (compatible; TelinhaBot/1.0; +link preview)';
const IMAGE_TYPES = new Set(['image/png', 'image/jpeg', 'image/webp', 'image/gif']);

const ENTITIES = {
  amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ', hellip: '…', mdash: '—', ndash: '–',
  laquo: '«', raquo: '»', ldquo: '“', rdquo: '”', lsquo: '‘', rsquo: '’', middot: '·', bull: '•',
  copy: '©', reg: '®', trade: '™', deg: '°', ordf: 'ª', ordm: 'º', iexcl: '¡', iquest: '¿',
  aacute: 'á', eacute: 'é', iacute: 'í', oacute: 'ó', uacute: 'ú', Aacute: 'Á', Eacute: 'É', Iacute: 'Í',
  Oacute: 'Ó', Uacute: 'Ú', agrave: 'à', Agrave: 'À', acirc: 'â', ecirc: 'ê', ocirc: 'ô', Acirc: 'Â',
  Ecirc: 'Ê', Ocirc: 'Ô', atilde: 'ã', otilde: 'õ', Atilde: 'Ã', Otilde: 'Õ', ccedil: 'ç', Ccedil: 'Ç',
  uuml: 'ü', Uuml: 'Ü', ntilde: 'ñ', Ntilde: 'Ñ',
};

function decodeEntities(text) {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z][a-z0-9]*);/gi, (all, code) => {
    if (code.charAt(0) === '#') {
      const hex = code.charAt(1).toLowerCase() === 'x';
      const value = Number.parseInt(code.slice(hex ? 2 : 1), hex ? 16 : 10);
      return Number.isInteger(value) && value > 0 && value <= 0x10ffff ? String.fromCodePoint(value) : all;
    }
    return ENTITIES[code] ?? ENTITIES[code.toLowerCase()] ?? all;
  });
}

function cleanText(value, max) {
  const text = decodeEntities(String(value ?? '')).replace(/\s+/g, ' ').trim();
  return text.length > max ? `${text.slice(0, max - 1).trimEnd()}…` : text;
}

function parseAttributes(tag) {
  const attributes = {};
  const body = tag.replace(/^<\s*[a-z]+/i, '').replace(/\/?>$/, '');
  const pattern = /([^\s=/>"']+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g;
  for (const match of body.matchAll(pattern)) {
    const name = match[1].toLowerCase();
    if (!(name in attributes)) {
      attributes[name] = match[2] ?? match[3] ?? match[4] ?? '';
    }
  }
  return attributes;
}

function extractMeta(html) {
  const end = html.search(/<\/head\s*>/i);
  const head = end >= 0 ? html.slice(0, end) : html;
  const meta = new Map();
  for (const [tag] of head.matchAll(/<meta\b[^>]*>/gi)) {
    const attributes = parseAttributes(tag);
    const key = String(attributes.property || attributes.name || attributes.itemprop || '').toLowerCase();
    if (key && attributes.content && !meta.has(key)) {
      meta.set(key, attributes.content);
    }
  }
  const title = /<title[^>]*>([\s\S]*?)<\/title\s*>/i.exec(head);
  return { meta, title: title ? title[1] : '' };
}

function decodeHtml(buffer, contentType) {
  let charset = /charset=["']?([\w-]+)/i.exec(contentType)?.[1];
  if (!charset) {
    charset = /<meta[^>]+charset=["']?([\w-]+)/i.exec(buffer.subarray(0, 4096).toString('latin1'))?.[1];
  }
  try {
    return new TextDecoder(charset || 'utf-8').decode(buffer);
  } catch {
    return new TextDecoder('utf-8').decode(buffer);
  }
}

function isPrivateAddress(address) {
  if (net.isIPv4(address)) {
    const [a, b] = address.split('.').map(Number);
    return a === 0 || a === 10 || a === 127 || a >= 224 || (a === 169 && b === 254)
      || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 100 && b >= 64 && b <= 127);
  }
  if (net.isIPv6(address)) {
    const lower = address.toLowerCase();
    const mapped = /^::ffff:(\d+\.\d+\.\d+\.\d+)$/.exec(lower);
    if (mapped) {
      return isPrivateAddress(mapped[1]);
    }
    return lower === '::' || lower === '::1' || /^f[cd]/.test(lower) || /^fe[89ab]/.test(lower) || /^ff/.test(lower);
  }
  return true;
}

// A previa nunca busca enderecos da rede local: uma mensagem nao pode fazer o computador de
// ninguem mandar pedidos para o roteador ou outras maquinas da casa.
async function assertPublic(url) {
  const host = url.hostname.replace(/^\[|\]$/g, '').toLowerCase();
  if (net.isIP(host)) {
    if (isPrivateAddress(host)) {
      throw new Error('endereco local');
    }
    return;
  }
  if (!host.includes('.') || host.endsWith('.localhost') || host.endsWith('.local') || host.endsWith('.lan')) {
    throw new Error('endereco local');
  }
  const addresses = await dns.lookup(host, { all: true, verbatim: true });
  if (addresses.length === 0 || addresses.some(({ address }) => isPrivateAddress(address))) {
    throw new Error('endereco local');
  }
}

async function request(url, accept, signal) {
  let current = url;
  for (let hop = 0; hop <= MAX_REDIRECTS; hop += 1) {
    if (current.protocol !== 'http:' && current.protocol !== 'https:') {
      throw new Error('protocolo nao suportado');
    }
    await assertPublic(current);
    const response = await fetch(current, {
      redirect: 'manual',
      signal,
      headers: {
        'user-agent': USER_AGENT,
        accept,
        'accept-language': 'pt-BR,pt;q=0.9,en;q=0.7',
      },
    });
    const location = response.headers.get('location');
    if (response.status >= 300 && response.status < 400 && location) {
      response.body?.cancel().catch(() => {});
      current = new URL(location, current);
      continue;
    }
    return { response, url: current };
  }
  throw new Error('redirecionamentos demais');
}

async function readBody(response, limit, untilHeadEnds) {
  if (!response.body) {
    return { data: Buffer.alloc(0), complete: true };
  }
  const reader = response.body.getReader();
  const chunks = [];
  let total = 0;
  let complete = true;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    chunks.push(value);
    total += value.length;
    if (total >= limit) {
      complete = false;
      break;
    }
    if (untilHeadEnds && /<\/head\s*>/i.test(Buffer.from(value).toString('latin1'))) {
      break;
    }
  }
  reader.cancel().catch(() => {});
  return { data: Buffer.concat(chunks, Math.min(total, limit)), complete };
}

function imageType(response) {
  const type = String(response.headers.get('content-type') ?? '').split(';')[0].trim().toLowerCase();
  return IMAGE_TYPES.has(type) ? type : null;
}

async function readImage(response) {
  const mime = imageType(response);
  if (!response.ok || !mime) {
    return null;
  }
  const { data, complete } = await readBody(response, MAX_IMAGE_BYTES, false);
  return complete && data.length > 0 ? { data, mime } : null;
}

async function fetchImage(url, signal) {
  const { response } = await request(url, 'image/webp,image/png,image/jpeg,image/gif;q=0.9', signal);
  return readImage(response);
}

function siteOf(url) {
  return url.hostname.replace(/^www\./i, '');
}

function fileNameOf(url) {
  let name = url.pathname.split('/').filter(Boolean).pop() ?? '';
  try {
    name = decodeURIComponent(name);
  } catch {
    // escape invalido, o nome fica como veio no endereco
  }
  return cleanText(name, MAX_TITLE);
}

async function fetchPreview(address) {
  const url = new URL(address);
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
  try {
    const { response, url: landed } = await request(url,
      'text/html,application/xhtml+xml;q=0.9,image/*;q=0.8,*/*;q=0.5', controller.signal);
    if (!response.ok) {
      response.body?.cancel().catch(() => {});
      return null;
    }
    const contentType = String(response.headers.get('content-type') ?? '');
    if (imageType(response)) {
      const image = await readImage(response);
      return image ? {
        url: address, siteName: siteOf(landed), title: fileNameOf(landed), description: '', image,
      } : null;
    }
    if (!/html/i.test(contentType)) {
      response.body?.cancel().catch(() => {});
      return null;
    }

    const { data } = await readBody(response, MAX_HTML_BYTES, true);
    const { meta, title } = extractMeta(decodeHtml(data, contentType));
    const pick = (...keys) => keys.map((key) => meta.get(key)).find(Boolean) ?? '';
    const preview = {
      url: address,
      siteName: cleanText(pick('og:site_name', 'application-name') || siteOf(landed), MAX_SITE),
      title: cleanText(pick('og:title', 'twitter:title') || title, MAX_TITLE),
      description: cleanText(pick('og:description', 'twitter:description', 'description'), MAX_DESCRIPTION),
      image: null,
    };
    const imageAddress = decodeEntities(pick('og:image:secure_url', 'og:image:url', 'og:image',
      'twitter:image', 'twitter:image:src').trim());
    if (imageAddress) {
      try {
        preview.image = await fetchImage(new URL(imageAddress, landed), controller.signal);
      } catch {
        preview.image = null;
      }
    }
    return preview.title || preview.description || preview.image ? preview : null;
  } finally {
    clearTimeout(timer);
  }
}

class LinkPreviews {
  constructor() {
    this.cache = new Map();
  }

  get(address) {
    let url;
    try {
      url = new URL(String(address));
    } catch {
      return Promise.resolve(null);
    }
    if ((url.protocol !== 'http:' && url.protocol !== 'https:') || url.href.length > MAX_URL_LENGTH) {
      return Promise.resolve(null);
    }
    const key = String(address);
    const cached = this.cache.get(key);
    if (cached && Date.now() - cached.at < CACHE_TTL_MS) {
      return cached.promise;
    }
    const promise = fetchPreview(key).catch(() => null);
    this.cache.delete(key);
    this.cache.set(key, { at: Date.now(), promise });
    while (this.cache.size > CACHE_LIMIT) {
      this.cache.delete(this.cache.keys().next().value);
    }
    return promise;
  }
}

module.exports = {
  LinkPreviews, decodeEntities, extractMeta, isPrivateAddress,
};
