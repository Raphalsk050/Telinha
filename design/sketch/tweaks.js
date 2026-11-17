// Painel "Vidro" do esboço: controles que mexem nas variáveis de glass.css ao vivo.
// Os valores ficam no endereço junto com o resto do estado.
(() => {
  const KNOBS = [
    { key: "frost", label: "Jateamento", min: 0, max: 1, step: 0.01, def: 0, show: (v) => Math.round(v * 100) + "%" },
    { key: "glass-dark", label: "Escurecimento", min: 0, max: 1, step: 0.01, def: 0.49, show: (v) => Math.round(v * 100) + "%" },
    { key: "blur", label: "Desfoque", min: 0, max: 80, step: 1, def: 12, unit: "px", show: (v) => v + " px" },
    { key: "light", label: "Luz de fundo", min: 0, max: 1.6, step: 0.01, def: 1.03, show: (v) => Math.round(v * 100) + "%" },
    { key: "light-size", label: "Tamanho das luzes", min: 0.3, max: 2.5, step: 0.05, def: 1.2, show: (v) => v.toFixed(2) + "×" },
  ];
  const values = Object.assign(Object.fromEntries(KNOBS.map((k) => [k.key, k.def])), state.glass || {});

  const panel = document.getElementById("tweaks");
  panel.innerHTML = `
    <div class="tweaks-head" title="Arraste para mover">
      <span class="grip-dots"></span><strong>Vidro</strong>
      <button data-reset title="Voltar ao padrão">padrão</button>
      <button data-copy title="Copiar os valores como CSS">copiar</button>
      <button data-fold title="Recolher">–</button>
    </div>
    ${KNOBS.map((k) => `
      <label class="knob">
        <span>${k.label}<output id="out-${k.key}"></output></span>
        <input type="range" data-key="${k.key}" min="${k.min}" max="${k.max}" step="${k.step}">
      </label>`).join("")}`;

  function apply(save = true) {
    for (const k of KNOBS) {
      const v = Number(values[k.key]);
      document.body.style.setProperty("--" + k.key, v + (k.unit || ""));
      panel.querySelector(`[data-key="${k.key}"]`).value = v;
      panel.querySelector(`#out-${k.key}`).textContent = k.show(v);
    }
    if (save) {
      state.glass = { ...values };
      history.replaceState(null, "", "#" + encodeURIComponent(JSON.stringify(state)));
    }
  }

  panel.addEventListener("input", (e) => {
    const key = e.target.dataset.key;
    if (!key) return;
    values[key] = Number(e.target.value);
    apply();
  });
  panel.addEventListener("click", (e) => {
    if (e.target.dataset.reset !== undefined) {
      for (const k of KNOBS) values[k.key] = k.def;
      apply();
    } else if (e.target.dataset.copy !== undefined) {
      const css = KNOBS.map((k) => `  --${k.key}: ${values[k.key]}${k.unit || ""};`).join("\n");
      navigator.clipboard?.writeText(css).catch(() => {});
      e.target.textContent = "copiado";
      setTimeout(() => (e.target.textContent = "copiar"), 1200);
    }
  });

  // ---- janela: arrastar pelo título, recolher, lembrar onde ficou ----
  const head = panel.querySelector(".tweaks-head");
  function place(x, y) {
    const w = panel.offsetWidth, h = panel.offsetHeight;
    x = Math.max(0, Math.min(innerWidth - w, x));
    y = Math.max(0, Math.min(innerHeight - 40, y));
    Object.assign(panel.style, { left: x + "px", top: y + "px", right: "auto", bottom: "auto" });
    return { x, y };
  }
  function remember() {
    try { localStorage.setItem("tweaks-window", JSON.stringify({ x: panel.offsetLeft, y: panel.offsetTop, folded: panel.classList.contains("collapsed") })); } catch {}
  }
  head.addEventListener("pointerdown", (e) => {
    if (e.target.closest("button")) return;
    const dx = e.clientX - panel.offsetLeft, dy = e.clientY - panel.offsetTop;
    head.setPointerCapture(e.pointerId);
    panel.classList.add("dragging");
    const move = (ev) => place(ev.clientX - dx, ev.clientY - dy);
    const up = () => {
      head.removeEventListener("pointermove", move);
      head.removeEventListener("pointerup", up);
      panel.classList.remove("dragging");
      remember();
    };
    head.addEventListener("pointermove", move);
    head.addEventListener("pointerup", up);
  });
  head.addEventListener("dblclick", (e) => { if (!e.target.closest("button")) fold(); });
  function fold() {
    panel.classList.toggle("collapsed");
    panel.querySelector("[data-fold]").textContent = panel.classList.contains("collapsed") ? "+" : "–";
    remember();
  }
  panel.querySelector("[data-fold]").addEventListener("click", fold);
  addEventListener("resize", () => { if (panel.style.left) place(panel.offsetLeft, panel.offsetTop); });
  try {
    const saved = JSON.parse(localStorage.getItem("tweaks-window") || "null");
    if (saved) {
      requestAnimationFrame(() => place(saved.x, saved.y));
      if (saved.folded) fold();
    }
  } catch {}

  apply(false);
})();
