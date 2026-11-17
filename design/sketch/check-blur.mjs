// Confere se todo vidro do tema Vidro desfoca de verdade, inclusive depois que a interface muda.
//
//   node design/sketch/check-blur.mjs            (com o serve.py rodando na porta 5180)
//
// Abre o esboço no Edge headless com ?probe (listras atrás de cada vidro, conteúdo escondido),
// passa por várias situações (re-render, troca de CSS ao vivo, arrastar a janela, mexer nos
// controles, outras telas) e mede a nitidez das listras dentro de cada vidro: desfocado dá
// quase zero; sem desfoque dá alto. Cada medida é comparada com a mesma tela com todo
// backdrop-filter desligado, para provar que havia listras atrás. Sai com código 1 se algo falhar.

import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { inflateSync } from "node:zlib";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const SHOTS = join(HERE, "shots");
const URL_BASE = process.env.SKETCH_URL || "http://localhost:5180/design/sketch/";
const EDGE = process.env.EDGE || "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe";
const PORT = 9339;
const W = 1360, H = 860;
const BLURRED_MAX = 1.5; // nitidez máxima aceita num vidro
const RATIO_MAX = 0.3;   // e no máximo 30% da nitidez da mesma tela sem desfoque
const SHARP_MIN = 2.5;   // abaixo disso não havia listras atrás (ex.: vidro na frente de outro vidro)

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---- PNG mínimo (o que o Chromium devolve: 8 bits, RGB ou RGBA, sem entrelaçamento) ----
function decodePng(buf) {
  let pos = 8, width = 0, height = 0, type = 0;
  const idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos), kind = buf.toString("ascii", pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (kind === "IHDR") { width = data.readUInt32BE(0); height = data.readUInt32BE(4); type = data[9]; }
    else if (kind === "IDAT") idat.push(data);
    else if (kind === "IEND") break;
    pos += 12 + len;
  }
  const bpp = type === 6 ? 4 : 3, stride = width * bpp;
  const raw = inflateSync(Buffer.concat(idat)), out = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const f = raw[y * (stride + 1)], src = y * (stride + 1) + 1, dst = y * stride;
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? out[dst + x - bpp] : 0, b = y > 0 ? out[dst + x - stride] : 0;
      const c = x >= bpp && y > 0 ? out[dst + x - stride - bpp] : 0, v = raw[src + x];
      let p = v;
      if (f === 1) p = v + a; else if (f === 2) p = v + b; else if (f === 3) p = v + ((a + b) >> 1);
      else if (f === 4) { const q = a + b - c, pa = Math.abs(q - a), pb = Math.abs(q - b), pc = Math.abs(q - c); p = v + (pa <= pb && pa <= pc ? a : pb <= pc ? b : c); }
      out[dst + x] = p & 255;
    }
  }
  return { width, height, bpp, px: out };
}

// Nitidez horizontal: média da diferença de luminância entre pixels vizinhos.
function sharpness(img, box) {
  const lum = (x, y) => { const i = (y * img.width + x) * img.bpp; return 0.299 * img.px[i] + 0.587 * img.px[i + 1] + 0.114 * img.px[i + 2]; };
  const x0 = Math.max(0, Math.round(box.x)), x1 = Math.min(img.width - 1, Math.round(box.x + box.w));
  const y0 = Math.max(0, Math.round(box.y)), y1 = Math.min(img.height - 1, Math.round(box.y + box.h));
  let sum = 0, n = 0;
  for (let y = y0; y < y1; y += 2) for (let x = x0; x < x1 - 1; x++) { sum += Math.abs(lum(x + 1, y) - lum(x, y)); n++; }
  return n ? sum / n : 0;
}

// ---- CDP ----
async function connect() {
  const dir = mkdtempSync(join(tmpdir(), "blurcheck-"));
  const edge = spawn(EDGE, ["--headless=new", `--remote-debugging-port=${PORT}`, `--user-data-dir=${dir}`, "--hide-scrollbars", `--window-size=${W},${H}`, ...(process.env.NO_GPU ? [] : ["--enable-gpu", "--ignore-gpu-blocklist", "--use-angle=d3d11", "--enable-gpu-rasterization"]), "about:blank"], { stdio: "ignore" });
  let target;
  for (let i = 0; i < 50 && !target; i++) {
    await sleep(200);
    try { target = await (await fetch(`http://127.0.0.1:${PORT}/json/new?about:blank`, { method: "PUT" })).json(); } catch {}
  }
  if (!target) throw new Error("o Edge headless não abriu a porta de depuração");
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  let id = 0;
  const waiting = new Map();
  ws.onmessage = (e) => { const m = JSON.parse(e.data); if (m.id && waiting.has(m.id)) { waiting.get(m.id)(m); waiting.delete(m.id); } };
  const send = (method, params = {}) => new Promise((r, j) => {
    const n = ++id;
    waiting.set(n, (m) => (m.error ? j(new Error(method + ": " + m.error.message)) : r(m.result)));
    ws.send(JSON.stringify({ id: n, method, params }));
  });
  const close = () => { ws.close(); edge.kill(); try { rmSync(dir, { recursive: true, force: true }); } catch {} };
  return { send, close };
}

const { send, close } = await connect();
const js = async (expr) => (await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true })).result.value;
const shot = async () => decodePng(Buffer.from((await send("Page.captureScreenshot", { format: "png" })).data, "base64"));
const mouse = (type, x, y) => send("Input.dispatchMouseEvent", { type, x, y, button: "left", buttons: type === "mouseReleased" ? 0 : 1, clickCount: 1 });

await send("Page.enable");
await send("Emulation.setDeviceMetricsOverride", { width: W, height: H, deviceScaleFactor: 1, mobile: false });

let opened = 0;
async function open(view) {
  const state = encodeURIComponent(JSON.stringify({ view, theme: "glass", members: true }));
  await send("Page.navigate", { url: `${URL_BASE}?probe=1&n=${++opened}#${state}` });
  await sleep(900);
  // GLASS_CSS=arquivo.css troca o glass.css por outro (ex.: uma versão antiga, para comparar).
  if (process.env.GLASS_CSS) {
    await js(`(async () => { const l = [...document.querySelectorAll("link[rel=stylesheet]")].find((x) => x.href.includes("glass"));
      l.href = ${JSON.stringify(process.env.GLASS_CSS)}; await new Promise((r) => (l.onload = r)); })()`);
    await sleep(200);
  }
  // INJECT_CSS="..." acrescenta CSS na página: serve para provar que a checagem pega um vidro quebrado.
  if (process.env.INJECT_CSS) await js(`document.head.insertAdjacentHTML("beforeend", ${JSON.stringify("<style>" + process.env.INJECT_CSS + "</style>")})`);
  // A janela Vidro vai para cima da lista de mensagens, onde há listras atrás.
  await js(`(() => { const t = document.getElementById("tweaks"); t.classList.remove("collapsed");
    Object.assign(t.style, { left: "520px", top: "${view === "call" ? 560 : 120}px", right: "auto", bottom: "auto" }); })()`);
  await sleep(150);
}

const results = [];
async function measure(scenario) {
  await js("__probePrepare()");
  await sleep(60);
  const surfaces = await js("__probeSurfaces()");
  const on = await shot();
  await js(`document.documentElement.classList.add("probe-off")`);
  await sleep(120);
  const off = await shot();
  await js(`document.documentElement.classList.remove("probe-off")`);
  await sleep(120);
  let bad = false;
  for (const s of surfaces) {
    const a = sharpness(on, s), b = sharpness(off, s);
    const status = a > BLURRED_MAX || (b >= SHARP_MIN && a > b * RATIO_MAX) ? "FALHOU" : b < SHARP_MIN ? "ok (sem listras atrás)" : "ok";
    if (status === "FALHOU") bad = true;
    results.push({ scenario, surface: s.name, blurred: a.toFixed(2), unblurred: b.toFixed(2), status });
  }
  if (bad) {
    mkdirSync(SHOTS, { recursive: true });
    const name = "blur-falha-" + scenario.replace(/[^a-z0-9]+/gi, "-") + ".png";
    writeFileSync(join(SHOTS, name), Buffer.from((await send("Page.captureScreenshot", { format: "png" })).data, "base64"));
  }
}

try {
  await open("chat");
  await measure("chat ao abrir");

  await js(`set({ channel: "canal-dos-memes" })`);
  await sleep(200);
  await measure("chat depois de re-render");

  await js(`__swapCss()`);
  await sleep(15);
  await measure("durante troca de CSS ao vivo");
  await sleep(400);
  await measure("depois da troca de CSS");

  // arrastar a janela pelo título e medir com o botão ainda apertado
  const head = await js(`(() => { const r = document.querySelector(".tweaks-head").getBoundingClientRect(); return { x: r.left + 40, y: r.top + 20 }; })()`);
  await mouse("mousePressed", head.x, head.y);
  for (let i = 1; i <= 8; i++) await mouse("mouseMoved", head.x - i * 10, head.y + i * 12);
  await sleep(100);
  await measure("arrastando a janela");
  await mouse("mouseReleased", head.x - 80, head.y + 96);
  await sleep(100);
  await measure("depois de soltar a janela");

  await js(`(() => { const i = document.querySelector('#tweaks [data-key="blur"]'); i.value = 30; i.dispatchEvent(new Event("input", { bubbles: true })); })()`);
  await sleep(150);
  await measure("desfoque mudado para 30px");

  await open("call");
  await measure("sala de voz");
  await open("home");
  await measure("início");
  await open("settings");
  await measure("configurações");
} finally {
  close();
}

const pad = (s, n) => String(s).padEnd(n);
console.log(pad("situação", 30) + pad("vidro", 30) + pad("com blur", 10) + pad("sem blur", 10) + "resultado");
for (const r of results) console.log(pad(r.scenario, 30) + pad(r.surface, 30) + pad(r.blurred, 10) + pad(r.unblurred, 10) + r.status);
const failed = results.filter((r) => r.status === "FALHOU");
console.log(failed.length ? `\n${failed.length} medida(s) com problema; prints em ${SHOTS}` : `\nTodos os ${results.length} vidros desfocam.`);
process.exit(failed.length ? 1 : 0);
