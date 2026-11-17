// Dados de exemplo: os mesmos do teste de snapshot do app Rust.
const DATA = {
  me: { name: "Raphalsk", me: true, status: "sem conexão", style: "prism", bio: "Faço o Telinha.", version: "0.4.3" },
  servers: [
    { id: "sd", initials: "SD", name: "Servidor de rapha", online: true },
    { id: "h", initials: "H", name: "Hangout" },
  ],
  channels: {
    text: [
      { name: "geral" },
      { name: "canal-dos-memes", unread: 3 },
    ],
    voice: [
      { name: "Geral", people: [
        { name: "Raphalsk", me: true, style: "prism" },
        { name: "Keep", live: true },
        { name: "nanda", muted: true },
      ] },
      { name: "Jogatina", people: [] },
    ],
  },
  members: {
    online: [
      { name: "Keep", style: "gradient", activity: "Transmitindo em Geral" },
      { name: "nanda", style: "neon", activity: "Na sala Geral" },
      { name: "Raphalsk", me: true, style: "prism", activity: "Na sala Geral" },
    ],
    offline: [{ name: "pedro" }],
  },
  messages: [
    { author: "Keep", style: "gradient", time: "Hoje às 18:17", lines: ["alguém vai jogar hoje?", "to devendo o agiota, preciso de distração"] },
    { author: "nanda", style: "neon", time: "Hoje às 18:37",
      lines: ['eu vou, só terminar aqui. olha isso <a href="#">https://example.com/receita</a>'],
      embed: { site: "example.com", title: "Receita de bolo de cenoura", text: "Fofinho, com cobertura de chocolate, pronto em 40 minutos." } },
    { author: "Keep", style: "gradient", time: "Hoje às 18:55", lines: ["olha essa"], image: true },
    { author: "nanda", style: "neon", time: "Hoje às 19:04", lines: ["kkkkk", "boa"] },
  ],
  call: {
    channel: "Geral",
    tiles: [
      { name: "Raphalsk", me: true, picture: true },
      { name: "Keep", live: true, connecting: true },
      { name: "nanda", muted: true, connecting: true },
    ],
  },
  contacts: [
    { name: "Keep", status: "Online" },
    { name: "nanda", status: "Na sala Geral" },
    { name: "pedro", status: "Offline", offline: true },
  ],
  // Avisos que chegam com o app aberto (botão "+ aviso" na barra do esboço).
  notices: {
    dm: { from: "nanda", style: "neon", icon: "chat", where: "Mensagem direta",
      text: "chegou o bolo, vem buscar antes que o Keep coma tudo", timed: true, open: { view: "home" } },
    mention: { from: "Keep", style: "gradient", icon: "hash", where: "geral · Servidor de rapha",
      text: '<span class="mention-chip">@Raphalsk</span> entra na sala, só falta você pra começar', timed: true, open: { view: "chat", channel: "geral" } },
    call: { from: "Keep", style: "gradient", icon: "phone", where: "Chamada de voz", text: "está te ligando" },
    live: { from: "Keep", style: "gradient", icon: "screen", where: "Geral · Servidor de rapha",
      text: "começou a transmitir a tela", timed: true, open: { view: "call" } },
  },
};
