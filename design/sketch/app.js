// Esboço da interface do Telinha. Cada parte da tela é uma função que devolve HTML;
// mude à vontade, o navegador recarrega sozinho.

const ICONS = "../../client/app/assets/icons/";
const icon = (name, cls = "") => `<i class="icon ${cls}" style="--src:url('${ICONS}${name}.svg')"></i>`;

// O mesmo hash e a mesma lista de cores do app (theme.rs), para cada um manter a sua cor.
const AVATAR_COLORS = ["#5865f2", "#3ba55c", "#faa61a", "#ed4245", "#eb459e", "#9b59b6", "#1abc9c", "#e67e22"];
function avatarColor(seed) {
  let hash = 0;
  for (const ch of seed) hash = (Math.imul(hash, 31) + ch.charCodeAt(0)) >>> 0;
  return AVATAR_COLORS[hash % AVATAR_COLORS.length];
}
function avatar(person, size = 32, dot = null) {
  const face = person.picture || person.me
    ? `<span class="avatar picture" style="--size:${size}px"></span>`
    : `<span class="avatar" style="--size:${size}px;background:${avatarColor(person.name)}">${person.name[0].toUpperCase()}</span>`;
  if (!dot) return face;
  return `<span class="avatar-wrap">${face}<span class="dot ${dot}"></span></span>`;
}
const nameOf = (person, extra = "") => `<span class="name ${person.style ? "style-" + person.style : ""}">${person.name}${extra}</span>`;

// ---- estado (fica no endereço, então sobrevive ao recarregar) ----
const state = Object.assign(
  { view: "chat", theme: "glass", members: true, channel: "geral", page: "profile", mic: true, sound: true },
  JSON.parse(decodeURIComponent(location.hash.slice(1) || "{}") || "{}"),
);
function set(patch) {
  Object.assign(state, patch);
  history.replaceState(null, "", "#" + encodeURIComponent(JSON.stringify(state)));
  render();
}

// ---- partes da tela ----
function rail() {
  const home = state.view === "home";
  return `<nav class="rail">
    <button class="rail-btn home ${home ? "active" : ""}" data-go="home" title="Início">${icon("home", "big")}</button>
    <div class="rail-sep"></div>
    ${DATA.servers.map((s, i) => `
      <button class="rail-btn ${!home && i === 0 ? "active" : ""}" data-go="chat" title="${s.name}">
        ${s.initials}${s.online ? '<span class="dot online"></span>' : ""}
      </button>`).join("")}
    <button class="rail-btn add" title="Adicionar servidor">${icon("plus")}</button>
  </nav>`;
}

function serverSidebar() {
  const inCall = state.view === "call";
  const text = DATA.channels.text.map((c) => `
    <button class="channel ${!inCall && state.channel === c.name ? "selected" : ""} ${c.unread ? "unread" : ""}" data-channel="${c.name}">
      ${icon("hash")}<span class="label">${c.name}</span>${c.unread ? `<span class="badge">${c.unread}</span>` : ""}
    </button>`).join("");
  const voice = DATA.channels.voice.map((c) => {
    const here = inCall && c.name === DATA.call.channel;
    const people = (here ? c.people : []).map((p) => `
      <div class="voice-person">
        ${avatar(p, 20)}${nameOf(p)}
        ${p.live ? '<span class="live">AO VIVO</span>' : ""}
        ${p.muted ? icon("mic_off", "small muted-icon") : ""}
      </div>`).join("");
    return `<button class="channel voice ${here ? "selected connected" : ""}" data-voice="${c.name}">
        ${icon("speaker")}<span class="label">${c.name}</span>${here ? '<span class="timer">0:00</span>' : ""}
      </button>${people}`;
  }).join("");
  return `<aside class="sidebar">
    <header class="sidebar-head">Servidor de rapha ${icon("chevron_down", "small")}</header>
    <div class="sidebar-scroll">
      <div class="group">CANAIS DE TEXTO ${icon("plus", "small")}</div>
      ${text}
      <div class="group">CANAIS DE VOZ ${icon("plus", "small")}</div>
      ${voice}
    </div>
    ${voicePanel()}
    ${userPanel()}
  </aside>`;
}

function homeSidebar() {
  return `<aside class="sidebar">
    <header class="sidebar-head search"><input placeholder="Conectar com código"></header>
    <div class="sidebar-scroll">
      <div class="group">MENSAGENS DIRETAS</div>
      ${DATA.contacts.map((c) => `
        <button class="contact ${c.offline ? "offline" : ""}">
          ${avatar(c, 32, c.offline ? "offline" : "online")}
          <span class="contact-text">${nameOf(c)}<small>${c.status}</small></span>
        </button>`).join("")}
    </div>
    ${voicePanel()}
    ${userPanel()}
  </aside>`;
}

function voicePanel() {
  if (state.view !== "call" && state.view !== "home") return "";
  return `<div class="voice-panel">
    <div class="voice-row">
      <span class="voice-state">${icon("signal", "small")} Voz conectada</span>
      <button class="ghost" data-go="chat" title="Desconectar">${icon("hangup")}</button>
    </div>
    <small>Geral / Servidor de rapha</small>
    <div class="voice-buttons">
      <button>${icon("camera")}</button><button>${icon("screen")}</button>
      <button>${icon("soundboard")}</button><button>${icon("pulse")}</button>
    </div>
  </div>`;
}

function userPanel() {
  return `<div class="user-panel">
    <button class="user" data-settings>
      ${avatar(DATA.me, 32, "online")}
      <span class="contact-text">${nameOf(DATA.me)}<small>${DATA.me.status}</small></span>
    </button>
    <button class="ghost ${state.mic ? "" : "off"}" data-toggle="mic" title="Microfone">${icon(state.mic ? "mic" : "mic_off")}</button>
    <button class="ghost ${state.sound ? "" : "off"}" data-toggle="sound" title="Som">${icon(state.sound ? "headphones" : "deaf")}</button>
    <button class="ghost" data-settings title="Configurações">${icon("gear")}</button>
  </div>`;
}

function topbar(title, sub, kind = "hash", actions = true) {
  return `<header class="topbar">
    ${icon(kind, "muted")}<strong>${title}</strong>${sub ? `<span class="sub">${sub}</span>` : ""}
    <span class="spacer"></span>
    ${actions ? `<button class="ghost" title="Convidar">${icon("invite")}</button>
    <button class="ghost ${state.members ? "on" : ""}" data-toggle="members" title="Membros">${icon("members")}</button>` : ""}
  </header>`;
}

function message(m) {
  const author = { name: m.author, style: m.style };
  return `<article class="message">
    ${avatar(author, 40)}
    <div class="message-body">
      <div class="message-head">${nameOf(author)}<time>${m.time}</time></div>
      ${m.lines.map((l) => `<p>${l}</p>`).join("")}
      ${m.embed ? `<div class="embed"><small>${m.embed.site}</small><a href="#">${m.embed.title}</a><p>${m.embed.text}</p></div>` : ""}
      ${m.image ? '<div class="image"></div>' : ""}
    </div>
  </article>`;
}

function chat(channel, intro, voice = false) {
  return `<section class="chat">
    <div class="messages">
      <div class="intro-icon">${icon(voice ? "speaker" : "hash", "huge")}</div>
      <h1>${voice ? "Chat de " + channel : "Bem-vindo a #" + channel}</h1>
      <p class="intro">${intro}</p>
      ${voice ? "" : DATA.messages.map(message).join("")}
    </div>
    ${composer(voice ? channel : "#" + channel)}
  </section>`;
}

function composer(where) {
  return `<div class="dock"><div class="composer">
    <button class="ghost attach" title="Anexar">${icon("plus", "small")}</button>
    <textarea rows="1" placeholder="Conversar em ${where}"></textarea>
    <button class="ghost">${icon("send")}</button>
  </div>
  <div class="footnote">Sem conexão com o servidor de contatos. As mensagens não vão chegar agora.</div></div>`;
}

function stage() {
  const tiles = DATA.call.tiles.map((t) => `
    <div class="tile">
      ${t.live ? '<span class="live corner">AO VIVO</span>' : ""}
      ${t.connecting ? '<span class="connecting">conectando…</span>' : ""}
      ${avatar(t, 72)}
      <span class="tile-name">${t.muted ? icon("mic_off", "small danger") : ""}${t.name}${t.me ? " (você)" : ""}</span>
      ${t.live ? '<button class="primary small watch">Assistir</button>' : ""}
    </div>`).join("");
  return `<section class="stage">
    <div class="tiles">${tiles}</div>
    <div class="call-controls">
      <button class="round ${state.mic ? "" : "off"}" data-toggle="mic">${icon(state.mic ? "mic" : "mic_off")}</button>
      <button class="round">${icon("camera")}</button>
      <button class="round">${icon("screen")}</button>
      <button class="round danger" data-go="chat">${icon("hangup")}</button>
    </div>
    <div class="grip"></div>
  </section>`;
}

function members() {
  if (!state.members || state.view === "home") return "";
  const row = (p, offline) => `
    <div class="member ${offline ? "offline" : ""}">
      ${avatar(p, 32, offline ? "offline" : "online")}
      <span class="contact-text">${nameOf(p, p.me ? " (você)" : "")}${state.view === "call" && p.activity ? `<small>${p.activity}</small>` : ""}</span>
    </div>`;
  return `<aside class="members">
    <div class="group">ONLINE — ${DATA.members.online.length}</div>
    ${DATA.members.online.map((p) => row(p, false)).join("")}
    <div class="group">OFFLINE — ${DATA.members.offline.length}</div>
    ${DATA.members.offline.map((p) => row(p, true)).join("")}
  </aside>`;
}

function home() {
  const card = (ic, title, text) => `<button class="card">${icon(ic, "huge accent")}<strong>${title}</strong><p>${text}</p></button>`;
  return `<section class="home-page">
    <h2>O que você quer fazer?</h2>
    <div class="cards">
      ${card("share_big", "Compartilhar com código", "Para a primeira vez com alguém: escolha a tela e mande o código para a outra pessoa.")}
      ${card("watch_big", "Assistir com código", "Cole o código que você recebeu e mande a resposta de volta.")}
      ${card("server_big", "Criar ou entrar num servidor", "Canais de texto e salas de voz para um grupo, com várias telas ao mesmo tempo.")}
    </div>
    <p class="footnote">Depois da primeira conexão a pessoa fica salva à esquerda. Clique no nome para conversar, ligar ou transmitir sem código.</p>
  </section>`;
}

function settings() {
  const pages = [
    ["USUÁRIO"], ["profile", "person", "Perfil"],
    ["APP"], ["voice", "mic", "Voz e vídeo"], ["appearance", "palette", "Aparência"], ["messages", "chat", "Mensagens"],
    ["TELINHA"], ["updates", "refresh", "Atualizações"],
  ];
  const nav = pages.map(([key, ic, label]) => !ic
    ? `<div class="group">${key}</div>`
    : `<button class="settings-link ${state.page === key ? "selected" : ""}" data-page="${key}">${icon(ic)}${label}</button>`).join("");
  const title = pages.find((p) => p[0] === state.page)?.[2] || "";
  return `<div class="overlay" data-close>
    <div class="settings" role="dialog">
      <nav class="settings-nav">
        <div class="settings-me">${avatar(DATA.me, 36)}<span class="contact-text"><strong>${DATA.me.name}</strong><small>Editar perfil</small></span></div>
        ${nav}
        <span class="spacer"></span>
        <small class="muted">Telinha ${DATA.me.version}</small>
      </nav>
      <div class="settings-page">
        <header><h2>${title}</h2><button class="ghost" data-close>${icon("close")}</button></header>
        ${state.page === "profile" ? profilePage() : `<p class="muted">Esboço desta página ainda não feito.</p>`}
      </div>
    </div>
  </div>`;
}

function profilePage() {
  return `<div class="profile">
    <div class="field"><label>AVATAR</label>
      <div class="row">${avatar(DATA.me, 72)}<button class="primary">Mudar avatar</button><button class="link-btn">Remover avatar</button></div></div>
    <div class="field"><label>SEU NOME</label>
      <div class="row"><input value="${DATA.me.name}"><button class="primary">Salvar</button></div></div>
    <div class="profile-cols">
      <div class="field"><label>ESTILO DO NOME</label>
        <div class="segmented">${["Nenhum", "Sólido", "Degradê", "Neon", "Prisma"].map((s) => `<button class="${s === "Prisma" ? "on" : ""}">${s}</button>`).join("")}</div>
        <textarea class="bio">${DATA.me.bio}</textarea>
        <small class="muted">Quem estiver em uma conversa com você vê seu nome e seu cartão assim.</small>
      </div>
      <div class="profile-card">
        <div class="banner"></div>
        ${avatar(DATA.me, 80, "online")}
        <strong>${nameOf(DATA.me)}</strong><small>Online</small><hr>
        <b>Sobre mim</b><p>${DATA.me.bio}</p>
      </div>
    </div>
  </div>`;
}

// ---- montagem ----
function render() {
  document.body.dataset.theme = state.theme;
  const view = state.view;
  let side, main;
  if (view === "home") {
    side = homeSidebar();
    main = `${topbar("Início", "", "home", false)}<div class="content">${home()}</div>`;
  } else if (view === "call") {
    side = serverSidebar();
    main = `${topbar("Geral", "3 na sala", "speaker")}<div class="content">
      <div class="column">${stage()}${chat("Geral", "Converse aqui com quem está na sala. As mensagens ficam guardadas só neste computador.", true)}</div>${members()}</div>`;
  } else {
    side = serverSidebar();
    main = `${topbar(state.channel, "Servidor de rapha")}<div class="content">
      ${chat(state.channel, `Este é o começo do canal #${state.channel} em Servidor de rapha. Cada computador guarda o próprio histórico, e quem está offline não recebe.`)}${members()}</div>`;
  }
  // Configurações abre por cima do chat, como no app.
  document.getElementById("app").innerHTML = `${rail()}${side}<main class="main">${main}</main>${view === "settings" ? settings() : ""}`;
  document.getElementById("dev-view").value = view;
  document.getElementById("dev-theme").value = state.theme;
  document.getElementById("dev-members").checked = state.members;
  const list = document.querySelector(".messages");
  // A caixa de mensagem pode flutuar por cima da conversa (tema Vidro): a conversa reserva
  // embaixo a altura dela, que muda quando o texto cresce.
  for (const dock of document.querySelectorAll(".dock")) {
    new ResizeObserver(() => {
      dock.parentElement.style.setProperty("--dock-h", dock.offsetHeight + "px");
    }).observe(dock);
  }
  if (list) requestAnimationFrame(() => (list.scrollTop = list.scrollHeight));
}

document.addEventListener("click", (event) => {
  const el = event.target.closest("[data-go],[data-channel],[data-voice],[data-toggle],[data-settings],[data-page],[data-close]");
  if (!el) return;
  if (el.dataset.close !== undefined && event.target.closest("[data-close]") === el && (el.tagName === "BUTTON" || event.target === el)) {
    return set({ view: state.lastView || "chat" });
  }
  if (el.dataset.go) set({ view: el.dataset.go });
  else if (el.dataset.channel) set({ view: "chat", channel: el.dataset.channel });
  else if (el.dataset.voice) set({ view: "call" });
  else if (el.dataset.toggle) set({ [el.dataset.toggle]: !state[el.dataset.toggle] });
  else if (el.dataset.settings !== undefined) set({ lastView: state.view === "settings" ? state.lastView : state.view, view: "settings" });
  else if (el.dataset.page) set({ page: el.dataset.page });
});
document.addEventListener("keydown", (e) => { if (e.key === "Escape" && state.view === "settings") set({ view: state.lastView || "chat" }); });
document.getElementById("dev-view").onchange = (e) => set({ view: e.target.value });
document.getElementById("dev-theme").onchange = (e) => set({ theme: e.target.value });
document.getElementById("dev-members").onchange = (e) => set({ members: e.target.checked });

render();
