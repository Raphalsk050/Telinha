// Recarrega o esboço quando um arquivo muda. CSS troca no lugar, sem perder o estado.
(() => {
  let last = null;
  const dot = document.getElementById("dev-live");

  // A folha nova entra ao lado da velha e a velha só sai quando a nova carregou: trocar o href
  // direto deixa um quadro sem estilo, e nesse quadro o vidro some.
  function swapCss(stamp) {
    for (const old of document.querySelectorAll('link[rel="stylesheet"]')) {
      const fresh = old.cloneNode();
      fresh.href = old.getAttribute("href").split("?")[0] + "?" + stamp;
      fresh.onload = () => old.remove();
      fresh.onerror = () => fresh.remove();
      old.after(fresh);
    }
  }
  window.__swapCss = () => swapCss(Date.now());

  async function tick() {
    try {
      const now = await (await fetch("/__stamps", { cache: "no-store" })).json();
      dot.classList.remove("off");
      if (last && now.other !== last.other) return location.reload();
      if (last && now.css !== last.css) swapCss(now.css);
      last = now;
    } catch {
      dot.classList.add("off");
    }
    setTimeout(tick, 400);
  }
  tick();
})();
