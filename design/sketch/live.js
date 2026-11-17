// Recarrega o esboço quando um arquivo muda. CSS troca no lugar, sem perder o estado.
(() => {
  let last = null;
  const dot = document.getElementById("dev-live");
  async function tick() {
    try {
      const now = await (await fetch("/__stamps", { cache: "no-store" })).json();
      dot.classList.remove("off");
      if (last && now.other !== last.other) return location.reload();
      if (last && now.css !== last.css) {
        for (const link of document.querySelectorAll('link[rel="stylesheet"]')) {
          link.href = link.getAttribute("href").split("?")[0] + "?" + now.css;
        }
      }
      last = now;
    } catch {
      dot.classList.add("off");
    }
    setTimeout(tick, 400);
  }
  tick();
})();
