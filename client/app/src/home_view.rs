//! The first screen, and the connection with codes that starts from it: the way two people
//! find each other the first time, after which each is a contact of the other.

use egui::{Align2, Color32, CornerRadius, FontId, Frame, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};
use serde_json::{Value, json};
use telinha_core::hub::{ManualRole, ShareChoice};

use crate::app::{Nav, TelinhaApp};
use crate::dialogs::{Dialog, Tone, button};
use crate::icons;
use crate::theme::{self, Palette};

const SHARE_STEPS: [&str; 4] = ["Preparando", "Mande o convite", "Cole a resposta", "Conectando"];
const WATCH_STEPS: [&str; 4] = ["Preparando", "Cole o convite", "Mande a resposta", "Conectando"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Starting,
    Live,
    Failed,
    Ended,
}

/// A connection with codes, from the first click until the person leaves its screen.
pub struct ManualSession {
    role: ManualRole,
    phase: Phase,
    running: bool,
    stopping: bool,
    status: String,
    step: usize,
    /// The code to send to the other person, and whether it is the invite.
    code_out: Option<(String, bool)>,
    copy_feedback: String,
    /// Set when the engine waits for a code. True when the code it waits for is the invite.
    code_in: Option<bool>,
    typed: String,
    submitted: bool,
    code_error: String,
    /// Why it failed or how it ended.
    message: String,
    feedback: String,
    target_name: String,
    saved: Option<(String, bool)>,
    fullscreen: bool,
}

impl ManualSession {
    fn new(role: ManualRole, target_name: String) -> Self {
        Self {
            role,
            phase: Phase::Starting,
            running: true,
            stopping: false,
            status: "Abrindo o Telinha…".to_owned(),
            step: 0,
            code_out: None,
            copy_feedback: String::new(),
            code_in: None,
            typed: String::new(),
            submitted: false,
            code_error: String::new(),
            message: String::new(),
            feedback: String::new(),
            target_name,
            saved: None,
            fullscreen: false,
        }
    }

    fn failed(role: ManualRole, message: String) -> Self {
        Self { phase: Phase::Failed, running: false, message, ..Self::new(role, String::new()) }
    }

    fn hide_codes(&mut self) {
        self.code_out = None;
        self.code_in = None;
        self.typed.clear();
        self.submitted = false;
        self.code_error.clear();
        self.copy_feedback.clear();
    }

    fn fail(&mut self, message: String) {
        if self.phase != Phase::Failed {
            self.phase = Phase::Failed;
            self.message = message;
            self.feedback.clear();
            self.hide_codes();
        }
    }

    /// Takes one event of the engine. True when the code that came out should be copied.
    pub fn engine_event(&mut self, event: &Value) -> bool {
        let text = |name: &str| event.get(name).and_then(Value::as_str).unwrap_or("");
        match text("event") {
            "step" => {
                if self.phase == Phase::Starting {
                    self.status = format!("{}…", capitalize(text("text")));
                }
            }
            "code" => {
                let invite = text("kind") == "invite";
                self.code_out = Some((text("code").to_owned(), invite));
                self.copy_feedback = "Copiado! Agora é só colar na conversa.".to_owned();
                if invite {
                    self.status = "Convite pronto.".to_owned();
                    self.step = 1;
                } else {
                    self.status = "Esperando a outra pessoa colar a resposta…".to_owned();
                    self.code_in = None;
                    self.step = 2;
                }
                return true;
            }
            "need_code" => {
                let invite = text("kind") == "invite";
                self.code_in = Some(invite);
                self.submitted = false;
                if invite {
                    self.status = "Pronto para receber o convite.".to_owned();
                    self.step = 1;
                } else {
                    self.status = "Esperando a resposta da outra pessoa…".to_owned();
                    self.step = 2;
                }
            }
            "code_rejected" => {
                self.submitted = false;
                self.code_error = describe_rejection(text("message"), self.role).to_owned();
            }
            "state" => match text("state") {
                "Connecting" => {
                    self.status = "Conectando os dois computadores…".to_owned();
                    self.step = 3;
                }
                "Connected" => {
                    self.phase = Phase::Live;
                    self.feedback.clear();
                    self.hide_codes();
                }
                "Disconnected" if self.phase == Phase::Live => {
                    self.feedback = "A conexão oscilou, tentando manter…".to_owned();
                }
                _ => {}
            },
            "target" => {
                let size = |name: &str| event.get(name).and_then(Value::as_i64).unwrap_or(0);
                self.feedback = format!("Agora mostrando a nova tela em {}×{}.", size("width"), size("height"));
            }
            "fullscreen" => self.fullscreen = event.get("enabled").and_then(Value::as_bool).unwrap_or(false),
            "command_failed" => {
                self.feedback = if text("command") == "switch_target" {
                    "Não consegui trocar para essa tela, continuo mostrando a anterior.".to_owned()
                } else {
                    format!("Não consegui mudar isso ({}).", text("message"))
                };
            }
            "error" => {
                if text("stage") == "audio" && self.phase == Phase::Live {
                    self.feedback = format!("Som: {}.", text("message"));
                } else if !self.stopping {
                    self.fail(describe_error(text("stage"), text("status"), text("message")));
                }
            }
            _ => {}
        }
        false
    }

    pub fn engine_ended(&mut self, code: Option<i32>) {
        self.running = false;
        let ended = |session: &mut Self, message: &str| {
            session.phase = Phase::Ended;
            session.message = message.to_owned();
            session.feedback.clear();
        };
        if self.stopping {
            ended(self, "Você encerrou a sessão.");
        } else if self.phase != Phase::Failed {
            match (code, self.role) {
                (Some(0), ManualRole::Watch) => ended(self, "A janela do vídeo foi fechada."),
                (Some(0), ManualRole::Share) => ended(self, "A transmissão terminou."),
                (code, _) => self.fail(format!("O Telinha fechou inesperadamente (código {}).", code.map_or_else(|| "?".to_owned(), |code| code.to_string()))),
            }
        }
        self.stopping = false;
        self.fullscreen = false;
        self.hide_codes();
    }

    pub(crate) fn active(&self) -> bool {
        matches!(self.phase, Phase::Starting | Phase::Live)
    }

    pub(crate) fn live(&self) -> bool {
        self.phase == Phase::Live
    }

    pub(crate) fn role(&self) -> ManualRole {
        self.role
    }

    pub(crate) fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub(crate) fn stopping(&self) -> bool {
        self.stopping
    }
}

fn capitalize(text: &str) -> String {
    let mut letters = text.chars();
    letters.next().map_or_else(String::new, |first| first.to_uppercase().chain(letters).collect())
}

fn describe_rejection(message: &str, role: ManualRole) -> &'static str {
    if message.contains("mesma ponta") {
        match role {
            ManualRole::Share => "Esse é o seu próprio convite. Cole a resposta que a outra pessoa mandou.",
            ManualRole::Watch => "Esse código é uma resposta, não um convite. Peça o convite para quem vai compartilhar.",
        }
    } else if message.contains("truncated") || message.contains("corrupted") {
        "O código chegou cortado ou alterado. Peça para a outra pessoa copiar de novo."
    } else if message.contains("another version") {
        "Esse código é de outra versão do Telinha. Os dois lados precisam da mesma versão."
    } else {
        "Esse código não funcionou. Confira se copiou o texto inteiro."
    }
}

fn describe_error(stage: &str, status: &str, message: &str) -> String {
    match stage {
        "network" => "Não foi possível abrir o caminho direto até a outra máquina. Redirecione as portas UDP 50000 a 50039 no roteador, ou coloquem os computadores na mesma rede do ZeroTier.".to_owned(),
        "connection_lost" => "A conexão caiu e não volta sozinha. Comecem de novo.".to_owned(),
        "audio" => format!("Som: {message}. Escolha \"Som do computador\" ou \"Sem som\" e tente de novo."),
        "prepare" => format!("Não consegui preparar o Telinha nesta máquina ({status}: {message})."),
        "target_closed" => "A janela que você estava transmitindo foi fechada, então a transmissão parou.".to_owned(),
        _ => format!("O Telinha parou com erro ({status}: {message})."),
    }
}

impl TelinhaApp {
    /// Ends the connection with codes, from its screen or from the call panel.
    pub(crate) fn stop_manual_session(&mut self) {
        let Some(manual) = &mut self.manual else { return };
        if manual.running && !manual.stopping {
            manual.stopping = true;
            manual.feedback = "Encerrando…".to_owned();
            self.hub.stop_manual();
        }
    }

    pub(crate) fn set_manual_fullscreen(&self, enabled: bool) {
        self.hub.manual_command(ManualRole::Watch, &json!({ "command": "set_fullscreen", "enabled": enabled }));
    }

    pub(crate) fn start_manual(&mut self, role: ManualRole, choice: Option<ShareChoice>) {
        let target = choice.as_ref().map_or_else(String::new, |choice| choice.name.clone());
        let contact_name = format!("Contato de {}", chrono::Local::now().format("%d/%m/%Y"));
        let started = {
            let _runtime = self.runtime.enter();
            self.hub.start_manual(role, choice, &contact_name)
        };
        self.manual = Some(match started {
            Ok(()) => ManualSession::new(role, target),
            Err(error) => ManualSession::failed(role, format!("{}.", capitalize(&error))),
        });
        self.navigate(Nav::Home);
    }

    pub(crate) fn manual_event(&mut self, event: &Value) {
        if self.room.shared.is_some_and(|shared| shared.manual) {
            self.stream_feedback(event);
        }
        self.debug_engine_event("manual", event);
        let Some(manual) = &mut self.manual else { return };
        if manual.engine_event(event) {
            if let Some((code, _)) = &manual.code_out {
                self.context.copy_text(code.clone());
            }
        }
    }

    pub(crate) fn manual_ended(&mut self, code: Option<i32>) {
        self.debug.remove("manual");
        if let Some(manual) = &mut self.manual {
            manual.engine_ended(code);
        }
        if self.room.picker_is_manual() {
            self.room.close_picker();
        }
        if self.room.shared.is_some_and(|shared| shared.manual) {
            self.room.shared = None;
        }
    }

    pub(crate) fn contact_saved(&mut self, contact_id: String, created: bool) {
        if let Some(manual) = &mut self.manual {
            manual.saved = Some((contact_id, created));
        }
    }

    /// Shows a made-up connection with codes, at the step where the invite is ready. For the
    /// tests.
    pub fn manual_for_test(&mut self, events: &[Value]) {
        let mut manual = ManualSession::new(ManualRole::Share, "Tela 1".to_owned());
        for event in events {
            manual.engine_event(event);
        }
        self.manual = Some(manual);
    }

    pub(crate) fn home_view(&mut self, ui: &mut Ui) {
        let palette = self.palette;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            // As `.home`: a column of at most 900 pixels in the middle, with 32 by 24 of padding.
            let column = ui.available_width().min(900.0);
            let width = (column - 48.0).max(200.0);
            let side = (ui.available_width() - column) / 2.0 + 24.0;
            ui.add_space(32.0);
            ui.horizontal_top(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    if self.manual.is_some() {
                        self.session_view(ui, &palette);
                    } else {
                        self.choices(ui, &palette);
                    }
                });
            });
            ui.add_space(32.0);
        });
    }

    fn choices(&mut self, ui: &mut Ui, palette: &Palette) {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(RichText::new("O que você quer fazer?").color(palette.strong).size(22.0).family(theme::bold()));
        ui.add_space(16.0);
        let mut picked = None;
        let cards = [
            (icons::SHARE_BIG, "Compartilhar com código", "Para a primeira vez com alguém: escolha a tela e mande o código para a outra pessoa."),
            (icons::WATCH_BIG, "Assistir com código", "Cole o código que você recebeu e mande a resposta de volta."),
            (icons::SERVER_BIG, "Criar ou entrar num servidor", "Canais de texto e salas de voz para um grupo, com várias telas ao mesmo tempo."),
        ];
        // As `.choice-grid`: as many columns of at least 240 pixels as fit, 16 apart.
        let width = ui.available_width();
        let columns = (((width + 16.0) / 256.0).floor() as usize).clamp(1, cards.len());
        let card_width = ((width - 16.0 * (columns - 1) as f32) / columns as f32).floor();
        let text_width = card_width - 50.0;
        let laid: Vec<_> = cards
            .iter()
            .map(|(_, title, text)| {
                let title = ui.painter().layout(title.to_string(), FontId::new(17.0, theme::semibold()), palette.strong, text_width);
                let mut job = egui::text::LayoutJob::default();
                job.wrap.max_width = text_width;
                job.append(text, 0.0, egui::TextFormat { font_id: FontId::proportional(15.0), color: palette.muted, line_height: Some(21.0), ..Default::default() });
                (title, ui.painter().layout_job(job))
            })
            .collect();
        for (row, chunk) in laid.chunks(columns).enumerate() {
            // A row is as tall as its tallest card, as a grid row is.
            let height = chunk.iter().map(|(title, text)| 1.0 + 24.0 + 44.0 + 8.0 + title.size().y + 8.0 + text.size().y + 24.0 + 1.0).fold(0.0, f32::max);
            let (line, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
            for (column, (title, text)) in chunk.iter().enumerate() {
                let index = row * columns + column;
                let rect = Rect::from_min_size(line.min + Vec2::new(column as f32 * (card_width + 16.0), 0.0), Vec2::new(card_width, height));
                let response = ui.interact(rect, ui.scope_id().with(("choice", index)), Sense::click());
                let hovered = response.hovered();
                if palette.glass {
                    crate::glass::glass_card(ui.painter(), rect, hovered, palette);
                } else {
                    ui.painter().rect_filled(rect, CornerRadius::same(8), if hovered { palette.hover } else { palette.sidebar });
                    ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, if hovered { palette.accent } else { palette.line_soft }), egui::StrokeKind::Inside);
                }
                let inner = rect.shrink(25.0);
                cards[index].0.image(44.0, palette.accent).paint_at(ui, Rect::from_min_size(inner.min, Vec2::splat(44.0)));
                let title_top = inner.min + Vec2::new(0.0, 44.0 + 8.0);
                ui.painter().galley(title_top, title.clone(), palette.strong);
                ui.painter().galley(title_top + Vec2::new(0.0, title.size().y + 8.0), text.clone(), palette.muted);
                if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    picked = Some(index);
                }
            }
            ui.add_space(16.0);
        }
        ui.add_space(20.0 - 16.0);
        ui.add(
            egui::Label::new(
                RichText::new("Depois da primeira conexão a pessoa fica salva à esquerda. Clique no nome para conversar, ligar ou transmitir sem código.")
                    .color(palette.muted)
                    .size(13.0),
            )
            .wrap(),
        );
        match picked {
            Some(0) => self.open_picker(false, true),
            Some(1) => self.start_manual(ManualRole::Watch, None),
            Some(2) => self.dialog = Some(Dialog::server(&self.hub.profile_name())),
            _ => {}
        }
    }

    fn session_view(&mut self, ui: &mut Ui, palette: &Palette) {
        let Some(manual) = &mut self.manual else { return };
        let share = manual.role == ManualRole::Share;
        let title = if share { "Compartilhando sua tela" } else { "Assistindo a tela de outra pessoa" };
        ui.label(RichText::new(title).color(palette.strong).size(24.0).family(theme::bold()));

        let mut submit = false;
        let mut stop = false;
        let mut dismiss = false;
        let mut switch = false;
        let mut settings = false;
        let mut debug = false;
        let mut fullscreen = None;
        let mut copy = None;

        if manual.phase == Phase::Starting {
            ui.label(RichText::new(manual.status.as_str()).color(palette.muted));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for (index, label) in (if share { SHARE_STEPS } else { WATCH_STEPS }).into_iter().enumerate() {
                    let (fill, ink) = match index.cmp(&manual.step) {
                        std::cmp::Ordering::Less => (palette.green, Color32::WHITE),
                        std::cmp::Ordering::Equal => (palette.accent, Color32::WHITE),
                        std::cmp::Ordering::Greater => (palette.raised, palette.muted),
                    };
                    let (dot, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 11.0, fill);
                    ui.painter().text(dot.center(), Align2::CENTER_CENTER, (index + 1).to_string(), FontId::new(12.0, theme::bold()), ink);
                    let color = if index == manual.step { palette.strong } else { palette.muted };
                    ui.label(RichText::new(label).color(color).size(14.0));
                    ui.add_space(8.0);
                }
            });
            ui.add_space(6.0);

            if let Some((code, invite)) = &manual.code_out {
                panel(ui, palette, |ui| {
                    let (heading, hint) = if *invite {
                        ("Mande este convite para quem vai assistir", "Ele já está copiado. Cole na conversa com a outra pessoa.")
                    } else {
                        ("Mande esta resposta de volta", "Ela já está copiada. Quando a outra pessoa colar, a conexão abre sozinha.")
                    };
                    ui.label(RichText::new(heading).color(palette.strong).size(16.0).family(theme::semibold()));
                    ui.label(RichText::new(hint).color(palette.muted).size(13.0));
                    Frame::new().fill(palette.rail).corner_radius(CornerRadius::same(4)).inner_margin(Margin::same(10)).show(ui, |ui| {
                        egui::ScrollArea::vertical().id_salt("code-out").max_height(96.0).auto_shrink([false, true]).show(ui, |ui| {
                            ui.add(egui::Label::new(RichText::new(code.as_str()).monospace().size(12.0).color(palette.text)).wrap());
                        });
                    });
                    ui.horizontal(|ui| {
                        if button(ui, palette, "Copiar código", Tone::Primary).clicked() {
                            copy = Some(code.clone());
                        }
                        ui.label(RichText::new(manual.copy_feedback.as_str()).color(palette.green).size(13.0));
                    });
                });
            }

            if let Some(invite) = manual.code_in {
                panel(ui, palette, |ui| {
                    let (heading, hint) = if invite {
                        ("Cole o convite que você recebeu", "Peça o convite para quem vai compartilhar a tela e cole aqui.")
                    } else {
                        ("Cole a resposta de quem vai assistir", "Quando a outra pessoa mandar a resposta, cole aqui para conectar.")
                    };
                    let label = ui.label(RichText::new(heading).color(palette.strong).size(16.0).family(theme::semibold()));
                    ui.label(RichText::new(hint).color(palette.muted).size(13.0));
                    ui.add_enabled_ui(!manual.submitted, |ui| {
                        Frame::new().fill(palette.rail).corner_radius(CornerRadius::same(4)).inner_margin(Margin::same(10)).show(ui, |ui| {
                            egui::ScrollArea::vertical().id_salt("code-in").max_height(96.0).show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut manual.typed)
                                        .frame(Frame::new())
                                        .font(egui::TextStyle::Monospace)
                                        .text_color(palette.text)
                                        .hint_text(RichText::new("TELINHA1.…").color(palette.muted))
                                        .desired_rows(3)
                                        .desired_width(f32::INFINITY),
                                )
                                .labelled_by(label.id);
                            });
                        });
                    });
                    if !manual.code_error.is_empty() {
                        ui.label(RichText::new(manual.code_error.as_str()).color(palette.danger_text).size(13.0));
                    }
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(!manual.submitted, |ui| {
                            if button(ui, palette, "Conectar", Tone::Primary).clicked() {
                                submit = true;
                            }
                            if button(ui, palette, "Colar da área de transferência", Tone::Plain).clicked() {
                                if let Ok(text) = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
                                    manual.typed = text.trim().to_owned();
                                    manual.code_error.clear();
                                }
                            }
                        });
                    });
                });
            }
            ui.add_space(4.0);
            if button(ui, palette, "Cancelar", Tone::Plain).clicked() {
                stop = true;
            }
        } else {
            let live = manual.phase == Phase::Live;
            let (heading, text) = match (manual.phase, share) {
                (Phase::Live, true) => ("Você está transmitindo", format!("Mostrando {}.", if manual.target_name.is_empty() { "sua tela" } else { &manual.target_name })),
                (Phase::Live, false) => ("Assistindo", "O vídeo abre numa janela do Telinha. F11, Alt+Enter ou dois cliques no vídeo alternam a tela cheia, Esc sai.".to_owned()),
                (Phase::Failed, _) => ("Não deu certo", manual.message.clone()),
                _ => ("Sessão encerrada", manual.message.clone()),
            };
            let saved = manual.saved.as_ref().map(|(id, created)| {
                let name = self.contacts.iter().find(|(contact, _)| &contact.id == id).map_or("contato novo", |(contact, _)| contact.name.as_str());
                if *created {
                    format!("Salvo nos contatos como “{name}”. Da próxima vez é só clicar no nome, sem código.")
                } else {
                    format!("Essa pessoa já está nos contatos como “{name}”.")
                }
            });
            panel(ui, palette, |ui| {
                let ink = match manual.phase {
                    Phase::Live => palette.green,
                    Phase::Failed => palette.danger_text,
                    _ => palette.strong,
                };
                ui.label(RichText::new(heading).color(ink).size(18.0).family(theme::semibold()));
                ui.label(RichText::new(text).color(palette.text));
                if live && !manual.feedback.is_empty() {
                    ui.label(RichText::new(manual.feedback.as_str()).color(palette.muted).size(13.0));
                }
                if let Some(saved) = saved {
                    ui.label(RichText::new(saved).color(palette.green).size(13.0));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if live && share && button(ui, palette, "Trocar a tela", Tone::Plain).clicked() {
                        switch = true;
                    }
                    if live && share && button(ui, palette, "Qualidade e som", Tone::Plain).clicked() {
                        settings = true;
                    }
                    if live && button(ui, palette, "Depuração", Tone::Plain).clicked() {
                        debug = true;
                    }
                    if live && !share {
                        let label = if manual.fullscreen { "Sair da tela cheia" } else { "Tela cheia" };
                        if button(ui, palette, label, Tone::Plain).clicked() {
                            fullscreen = Some(!manual.fullscreen);
                        }
                    }
                    if live {
                        ui.add_enabled_ui(!manual.stopping, |ui| {
                            if button(ui, palette, "Encerrar", Tone::Danger).clicked() {
                                stop = true;
                            }
                        });
                    } else if button(ui, palette, "Voltar ao início", Tone::Primary).clicked() {
                        dismiss = true;
                    }
                });
            });
        }

        if let Some(code) = copy {
            ui.ctx().copy_text(code);
            manual.copy_feedback = "Copiado! Agora é só colar na conversa.".to_owned();
        }
        if submit {
            manual.code_error.clear();
            if self.hub.submit_manual_code(&manual.typed) {
                manual.submitted = true;
                manual.status = if share { "Conectando…".to_owned() } else { "Gerando a resposta…".to_owned() };
                if share {
                    manual.step = 3;
                }
            } else {
                manual.code_error = "Isso não parece um código do Telinha. Copie o texto inteiro, que começa com TELINHA1.".to_owned();
            }
        }
        if stop && manual.running && !manual.stopping {
            manual.stopping = true;
            manual.feedback = "Encerrando…".to_owned();
            self.hub.stop_manual();
        }
        if let Some(enabled) = fullscreen {
            self.hub.manual_command(ManualRole::Watch, &json!({ "command": "set_fullscreen", "enabled": enabled }));
        }
        if dismiss && !manual.active() {
            self.manual = None;
        }
        if switch {
            self.open_picker(true, true);
        }
        if settings {
            self.open_stream_settings();
        }
        if debug {
            self.open_debug();
        }
    }
}

/// A block of the session screen, set apart from the page.
fn panel(ui: &mut Ui, palette: &Palette, content: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(palette.sidebar)
        .corner_radius(CornerRadius::same(8))
        .stroke(Stroke::new(1.0, palette.line_soft))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            content(ui);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn sharing_goes_from_the_invite_to_the_answer_to_live() {
        let mut manual = ManualSession::new(ManualRole::Share, "Tela 1".into());
        assert!(!manual.engine_event(&event(r#"{"event":"step","text":"preparando a captura"}"#)));
        assert_eq!(manual.status, "Preparando a captura…");
        assert!(manual.engine_event(&event(r#"{"event":"code","kind":"invite","code":"TELINHA1.abc"}"#)));
        assert_eq!((manual.step, manual.code_out.clone()), (1, Some(("TELINHA1.abc".to_owned(), true))));
        manual.engine_event(&event(r#"{"event":"need_code","kind":"answer"}"#));
        assert_eq!((manual.step, manual.code_in), (2, Some(false)));
        manual.submitted = true;
        manual.engine_event(&event(r#"{"event":"code_rejected","message":"codigo da mesma ponta"}"#));
        assert!(!manual.submitted);
        assert_eq!(manual.code_error, "Esse é o seu próprio convite. Cole a resposta que a outra pessoa mandou.");
        manual.engine_event(&event(r#"{"event":"state","state":"Connecting"}"#));
        assert_eq!(manual.step, 3);
        manual.engine_event(&event(r#"{"event":"state","state":"Connected"}"#));
        assert_eq!(manual.phase, Phase::Live);
        assert!(manual.code_out.is_none() && manual.code_in.is_none());
        manual.engine_ended(Some(0));
        assert_eq!((manual.phase, manual.message.as_str()), (Phase::Ended, "A transmissão terminou."));
    }

    #[test]
    fn an_error_or_a_crash_says_what_happened_and_a_stop_does_not_count_as_one() {
        let mut manual = ManualSession::new(ManualRole::Watch, String::new());
        manual.engine_event(&event(r#"{"event":"error","stage":"connection_lost","status":"x","message":"y"}"#));
        assert_eq!((manual.phase, manual.message.as_str()), (Phase::Failed, "A conexão caiu e não volta sozinha. Comecem de novo."));
        manual.engine_ended(Some(1));
        assert_eq!(manual.message, "A conexão caiu e não volta sozinha. Comecem de novo.");

        let mut crashed = ManualSession::new(ManualRole::Watch, String::new());
        crashed.engine_ended(Some(-1073741819));
        assert_eq!(crashed.message, "O Telinha fechou inesperadamente (código -1073741819).");

        let mut stopped = ManualSession::new(ManualRole::Share, String::new());
        stopped.stopping = true;
        stopped.engine_event(&event(r#"{"event":"error","stage":"prepare","status":"x","message":"y"}"#));
        stopped.engine_ended(Some(1));
        assert_eq!((stopped.phase, stopped.message.as_str()), (Phase::Ended, "Você encerrou a sessão."));
    }
}
