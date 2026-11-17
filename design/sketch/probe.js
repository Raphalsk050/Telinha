// Modo de prova do blur (só com ?probe na URL; usado pelo check-blur.mjs).
// Põe listras pretas e brancas atrás de cada vidro e esconde o conteúdo dos painéis: onde o vidro
// desfoca, as listras viram cinza liso; onde não desfoca, aparecem nítidas.
(() => {
  if (!new URLSearchParams(location.search).has("probe")) return;
  document.documentElement.classList.add("probe");
  const STRIPES = "repeating-linear-gradient(90deg, #fff 0 10px, #000 10px 20px)";
  const style = document.createElement("style");
  style.textContent = `
    html.probe body[data-theme="glass"]::before { background: ${STRIPES} !important; opacity: 1 !important; filter: none !important; }
    html.probe .messages > :not(.probe-stripes), html.probe #dev { visibility: hidden !important; }
    html.probe #app .probe-stripes { visibility: visible !important; color: #fff !important; font: 900 20px/20px "Segoe UI", sans-serif; word-break: break-all; }
    html.probe #app *, html.probe #tweaks * { color: transparent !important; text-shadow: none !important; caret-color: transparent !important; }
    html.probe :is(.icon, .avatar, .avatar-wrap, .dot, .badge, .live, .image, .embed, .grip-dots, input, select, textarea, hr, .connecting, .tile-name, .watch) { visibility: hidden !important; }
    html.probe .name { background: none !important; }
    html.probe *, html.probe *::before, html.probe *::after { transition: none !important; animation: none !important; }
    html.probe-off *, html.probe-off *::before, html.probe-off *::after { backdrop-filter: none !important; -webkit-backdrop-filter: none !important; }
  `;
  document.head.appendChild(style);

  const rect = (el, inset = {}) => {
    if (!el) return null;
    const r = el.getBoundingClientRect();
    const box = {
      x: r.left + (inset.l ?? 8), y: r.top + (inset.t ?? 8),
      w: r.width - (inset.l ?? 8) - (inset.r ?? 8), h: r.height - (inset.t ?? 8) - (inset.b ?? 8),
    };
    return box.w > 8 && box.h > 4 ? box : null;
  };

  // Listras feitas de TEXTO que rola dentro da lista (não um fundo): no Chromium do app o defeito
  // só aparecia com texto passando atrás da caixa de mensagem; listras desenhadas como fundo
  // continuavam desfocadas. Rola para elas ficarem atrás da caixa.
  window.__probePrepare = () => {
    for (const list of document.querySelectorAll(".messages")) {
      if (!list.querySelector(".probe-stripes")) {
        list.insertAdjacentHTML("beforeend", '<div class="probe-stripes"></div>');
        list.lastChild.textContent = "█▌ ".repeat(4000);
      }
      list.scrollTop = list.scrollHeight - list.clientHeight - 600;
    }
  };

  // Onde medir cada vidro: um pedaço sem nada desenhado por cima, só o vidro e o que está atrás.
  window.__probeSurfaces = () => {
    const list = [];
    const add = (name, box) => box && list.push({ name, ...box });
    const side = document.querySelector(".sidebar-scroll");
    if (side) { const r = side.getBoundingClientRect(); add("barra lateral", { x: r.left + 8, y: r.top + r.height * 0.6, w: r.width - 24, h: r.height * 0.35 }); }
    add("painel principal (membros)", rect(document.querySelector(".members"), { t: 220, r: 14 }));
    add("caixa de mensagem", rect(document.querySelector(".dock .composer"), { l: 44, r: 44, t: 10, b: 10 }));
    add("aviso embaixo", rect(document.querySelector(".dock .footnote"), { t: 3, b: 3, l: 16, r: 16 }));
    const tw = document.getElementById("tweaks");
    if (tw && getComputedStyle(tw).display !== "none") add("janela Vidro", rect(tw, { t: 48, l: 14, r: 14, b: 10 }));
    add("configurações (menu)", rect(document.querySelector(".settings-nav"), { t: 420, b: 60 }));
    document.querySelectorAll(".card").forEach((c, i) => add("cartão " + (i + 1), rect(c, { t: 16, b: 16, l: 16, r: 16 })));
    return list;
  };
})();
