//! The small windows that ask for something, drawn as the `.dialog` of the old page: a title,
//! sometimes a close button, the fields and a row of buttons.

use egui::{Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};
use telinha_core::servers::ChannelKind;

use crate::theme::{self, Palette};
use crate::widgets;

/// The width of a dialog, as `.dialog` had it: 460 pixels with 20 of padding on each side.
const INNER: f32 = 420.0;
const BUTTON: Color32 = Color32::from_rgb(0x4e, 0x50, 0x58);
const BUTTON_HOVER: Color32 = Color32::from_rgb(0x6d, 0x6f, 0x78);

/// What a prompt is for. The window decides what to do with the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    RenameServer(String),
    /// The relay of a server, written as `turn:address:port user password`.
    SetRelay(String),
    /// The name of a contact, with the way to remove it.
    Contact(String),
    RenameSound(String),
    /// The name of a sound just picked from a file, before it is kept.
    NameSound,
    ChangeName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Danger {
    LeaveServer(String),
    RemoveChannel(String, String),
    RemoveContact(String),
    DeleteMessage(String),
}

/// A contact the invite of a server can be sent to.
pub struct InviteContact {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub picture: Option<egui::Image<'static>>,
}

pub enum Dialog {
    /// "Criar ou entrar em um servidor": a name to create one, or an invite to join one.
    Server { name: String, code: String, error: String, focus: bool },
    /// One line of text, as `#prompt-dialog`.
    Prompt { purpose: Purpose, title: String, value: String, confirm: String, danger: Option<String>, error: String, focus: bool },
    /// "Criar canal" or "Editar canal".
    Channel { server_id: String, channel_id: Option<String>, kind: ChannelKind, name: String, focus: bool },
    /// The invite of a server, to copy or to send to a contact.
    Invite { server_name: String, code: String, feedback: String, sent: Vec<(String, String)> },
    Confirm { danger: Danger, title: String, text: String, confirm: String },
}

pub enum Answer {
    /// The dialog stays as it is.
    Open,
    Close,
    CreateServer(String),
    JoinServer(String),
    Text(Purpose, String),
    /// The red button of a prompt, such as "Remover" of a contact.
    Danger(Purpose),
    SaveChannel { server_id: String, channel_id: Option<String>, kind: ChannelKind, name: String },
    RemoveChannel(String, String),
    SendInvite(String),
    Confirmed(Danger),
}

impl Dialog {
    pub fn prompt(purpose: Purpose, title: &str, value: &str, confirm: &str, danger: Option<&str>) -> Self {
        Self::Prompt {
            purpose,
            title: title.to_owned(),
            value: value.to_owned(),
            confirm: confirm.to_owned(),
            danger: danger.map(str::to_owned),
            error: String::new(),
            focus: true,
        }
    }

    pub fn for_purpose(purpose: Purpose, current: &str) -> Self {
        match &purpose {
            Purpose::RenameServer(_) => Self::prompt(purpose, "Renomear servidor", current, "Salvar", None),
            Purpose::SetRelay(_) => {
                let danger = (!current.is_empty()).then_some("Remover relay");
                Self::prompt(purpose, "Relay das chamadas: turn:endereco:porta usuario senha", current, "Salvar", danger)
            }
            Purpose::Contact(_) => Self::prompt(purpose, "Contato", current, "Salvar", Some("Remover")),
            Purpose::RenameSound(_) => Self::prompt(purpose, "Renomear som", current, "Salvar", None),
            Purpose::NameSound => Self::prompt(purpose, "Nome do som", current, "Adicionar", None),
            Purpose::ChangeName => Self::prompt(purpose, "Seu nome", current, "Salvar", None),
        }
    }

    /// The dialog to create a server, its name already filled with the one of the person.
    pub fn server(person: &str) -> Self {
        let name: String = format!("Servidor de {person}").chars().take(40).collect();
        Self::Server { name, code: String::new(), error: String::new(), focus: true }
    }

    pub fn channel(server_id: &str, channel: Option<(&str, &str, ChannelKind)>, kind: ChannelKind) -> Self {
        Self::Channel {
            server_id: server_id.to_owned(),
            channel_id: channel.map(|(id, _, _)| id.to_owned()),
            kind: channel.map_or(kind, |(_, _, kind)| kind),
            name: channel.map_or_else(String::new, |(_, name, _)| name.to_owned()),
            focus: true,
        }
    }

    pub fn set_error(&mut self, text: &str) {
        match self {
            Self::Prompt { error, .. } | Self::Server { error, .. } => *error = text.to_owned(),
            _ => {}
        }
    }

    /// Marks an invite as sent to a contact, with what its button says now.
    pub fn invite_sent(&mut self, contact_id: &str, label: &str) {
        if let Self::Invite { sent, .. } = self {
            sent.retain(|(id, _)| id != contact_id);
            sent.push((contact_id.to_owned(), label.to_owned()));
        }
    }

    pub fn show(&mut self, context: &egui::Context, palette: &Palette, contacts: &[InviteContact]) -> Answer {
        let mut answer = Answer::Open;
        let response = crate::glass::modal(context, egui::Id::unique("dialog"), dialog_frame(palette), |ui| {
                ui.set_width(INNER);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                match self {
                    Self::Server { name, code, error, focus } => server(ui, palette, name, code, error, focus, &mut answer),
                    Self::Prompt { purpose, title, value, confirm, danger, error, focus } => {
                        let label = title_line(ui, palette, title);
                        let field = text_field_limited(ui, palette, value, 40).labelled_by(label);
                        if std::mem::take(focus) {
                            field.request_focus();
                            select_all(ui, &field, value);
                        }
                        if !error.is_empty() {
                            error_line(ui, palette, error);
                        }
                        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        let (danger_click, cancel, save) = actions(ui, palette, danger.as_deref(), confirm, Tone::Primary);
                        if danger_click {
                            answer = Answer::Danger(purpose.clone());
                        } else if cancel {
                            answer = Answer::Close;
                        } else if save || entered {
                            answer = Answer::Text(purpose.clone(), value.clone());
                        }
                    }
                    Self::Channel { server_id, channel_id, kind, name, focus } => {
                        let editing = channel_id.is_some();
                        if header(ui, palette, if editing { "Editar canal" } else { "Criar canal" }) {
                            answer = Answer::Close;
                        }
                        if !editing {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                for (label, choice) in [("# Texto", ChannelKind::Text), ("Voz", ChannelKind::Voice)] {
                                    if segment(ui, palette, label, *kind == choice) {
                                        *kind = choice;
                                    }
                                }
                            });
                        }
                        ui.add_space(12.0);
                        ui.label(RichText::new("Nome do canal").color(palette.muted).size(12.0));
                        ui.add_space(4.0);
                        let hint = if *kind == ChannelKind::Voice { "Nova sala" } else { "novo-canal" };
                        let field = text_field_hint(ui, palette, name, 32, hint);
                        if std::mem::take(focus) {
                            field.request_focus();
                        }
                        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        let (remove, cancel, save) = actions(ui, palette, editing.then_some("Excluir canal"), if editing { "Salvar" } else { "Criar canal" }, Tone::Primary);
                        if remove {
                            answer = Answer::RemoveChannel(server_id.clone(), channel_id.clone().unwrap_or_default());
                        } else if cancel {
                            answer = Answer::Close;
                        } else if save || entered {
                            answer = Answer::SaveChannel { server_id: server_id.clone(), channel_id: channel_id.clone(), kind: *kind, name: name.clone() };
                        }
                    }
                    Self::Invite { server_name, code, feedback, sent } => {
                        if header(ui, palette, &format!("Convidar para {server_name}")) {
                            answer = Answer::Close;
                        }
                        paragraph(ui, palette, "Quem tiver este convite entra no servidor. Mande só para quem você quer lá dentro.");
                        code_box(ui, palette, code);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            if button(ui, palette, "Copiar convite", Tone::Primary).clicked() {
                                ui.ctx().copy_text(code.clone());
                                *feedback = "Copiado! Mande para quem você quer no servidor.".to_owned();
                            }
                            ui.label(RichText::new(feedback.as_str()).color(palette.muted).size(13.0));
                        });
                        if !contacts.is_empty() {
                            section_title(ui, palette, "MANDAR PARA UM CONTATO");
                            egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                                for contact in contacts {
                                    let label = sent.iter().find(|(id, _)| id == &contact.id).map(|(_, label)| label.as_str());
                                    if invite_row(ui, palette, contact, label) {
                                        answer = Answer::SendInvite(contact.id.clone());
                                    }
                                }
                            });
                        }
                    }
                    Self::Confirm { danger, title, text, confirm } => {
                        let _ = title_line(ui, palette, title);
                        ui.add(egui::Label::new(RichText::new(text.as_str()).color(palette.muted).size(15.0)).wrap());
                        let (_, cancel, save) = actions(ui, palette, None, confirm, Tone::Danger);
                        if cancel {
                            answer = Answer::Close;
                        } else if save {
                            answer = Answer::Confirmed(danger.clone());
                        }
                    }
                }
            });
        if response.should_close() && matches!(answer, Answer::Open) {
            answer = Answer::Close;
        }
        answer
    }
}

/// The look of every dialog: the main color, rounded, with a shadow and no border.
/// The space between the edge of a dialog and what it holds.
pub const DIALOG_MARGIN: f32 = 20.0;

pub fn dialog_frame(palette: &Palette) -> Frame {
    Frame::new()
        .fill(palette.main)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(DIALOG_MARGIN as i8))
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(128) })
}

/// The title of a dialog with the close button at its right, as `.dialog-header`. True when
/// the close button was clicked.
pub fn header(ui: &mut Ui, palette: &Palette, title: &str) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        let room = ui.available_width() - 40.0;
        ui.allocate_ui_with_layout(Vec2::new(room, 32.0), Layout::left_to_right(Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(title).color(palette.strong).size(18.0).family(theme::bold())).truncate());
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            close = crate::debug_view::close_button(ui, palette);
        });
    });
    ui.add_space(8.0);
    close
}

/// The title of a dialog without a close button.
fn title_line(ui: &mut Ui, palette: &Palette, title: &str) -> egui::Id {
    let label = ui.add(egui::Label::new(RichText::new(title).color(palette.strong).size(18.0).family(theme::bold())).wrap());
    ui.add_space(8.0);
    label.id
}

/// A paragraph of `.muted.small` text, with the margins of a paragraph.
fn paragraph(ui: &mut Ui, palette: &Palette, text: &str) {
    ui.add_space(5.0);
    ui.add(egui::Label::new(RichText::new(text).color(palette.muted).size(13.0)).wrap());
    ui.add_space(13.0);
}

/// A `.section-title`: small bold capitals with room above.
pub fn section_title(ui: &mut Ui, palette: &Palette, text: &str) {
    ui.add_space(14.0);
    ui.label(RichText::new(text).color(palette.muted).size(12.0).family(theme::bold()));
    ui.add_space(8.0);
}

fn error_line(ui: &mut Ui, palette: &Palette, text: &str) {
    ui.add_space(6.0);
    ui.add(egui::Label::new(RichText::new(text).color(palette.danger_text).size(13.0)).wrap());
}

/// The code of an invite in a dark box, as `.code-box`.
fn code_box(ui: &mut Ui, palette: &Palette, code: &str) {
    ui.add_space(12.0);
    Frame::new().fill(palette.rail).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().max_height(116.0).auto_shrink([false, true]).show(ui, |ui| {
            let mut job = egui::text::LayoutJob::simple(code.to_owned(), FontId::monospace(12.0), palette.text, ui.available_width());
            job.wrap.break_anywhere = true;
            let galley = ui.painter().layout_job(job);
            let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
            ui.painter().galley(rect.min, galley, palette.text);
        });
    });
    ui.add_space(12.0);
}

/// The row of buttons at the foot of a dialog, as `.actions`: an optional red button on the
/// left, then "Cancelar" and the main one on the right. Returns which was clicked.
fn actions(ui: &mut Ui, palette: &Palette, danger: Option<&str>, confirm: &str, tone: Tone) -> (bool, bool, bool) {
    let (mut danger_click, mut cancel, mut save) = (false, false, false);
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.x = 8.0;
        if let Some(label) = danger {
            danger_click = button(ui, palette, label, Tone::Danger).clicked();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            save = button(ui, palette, confirm, tone).clicked();
            cancel = button(ui, palette, "Cancelar", Tone::Plain).clicked();
        });
    });
    (danger_click, cancel, save)
}

/// One choice of a `.segmented` group.
fn segment(ui: &mut Ui, palette: &Palette, label: &str, selected: bool) -> bool {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(13.0), palette.text);
    let (rect, response) = ui.allocate_exact_size(galley.size() + Vec2::new(24.0, 14.0), Sense::click());
    let (fill, ink) = if selected { (palette.accent, Color32::WHITE) } else { (palette.sidebar, palette.text) };
    ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A contact under "Mandar para um contato", with the button that sends the invite.
fn invite_row(ui: &mut Ui, palette: &Palette, contact: &InviteContact, label: Option<&str>) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), palette.hover);
    }
    let avatar = Rect::from_center_size(rect.left_center() + Vec2::new(4.0 + 16.0, 0.0), Vec2::splat(32.0));
    widgets::paint_avatar(ui, avatar, &contact.name, contact.picture.clone());
    widgets::status_dot(ui, avatar, contact.online, palette, if response.hovered() { palette.hover } else { palette.main });
    let galley = ui.painter().layout_no_wrap("Enviar".to_owned(), FontId::new(13.0, theme::semibold()), Color32::WHITE);
    let text = label.unwrap_or("Enviar");
    let galley = if text == "Enviar" { galley } else { ui.painter().layout_no_wrap(text.to_owned(), FontId::new(13.0, theme::semibold()), Color32::WHITE) };
    let send = Rect::from_min_size(egui::pos2(rect.right() - 4.0 - galley.size().x - 20.0, rect.center().y - 14.0), Vec2::new(galley.size().x + 20.0, 28.0));
    widgets::paint_text_clipped(ui, egui::pos2(avatar.right() + 10.0, rect.center().y), send.left() - avatar.right() - 20.0, &contact.name, FontId::proportional(15.0), palette.strong);
    let enabled = label.is_none();
    let click = ui.interact(send, ui.scope_id().with(("invite-send", &contact.id)), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if click.hovered() && enabled { palette.accent_hover } else { palette.accent };
    ui.painter().rect_filled(send, CornerRadius::same(4), if enabled { fill } else { fill.gamma_multiply(0.5) });
    ui.painter().galley(send.center() - galley.size() / 2.0, galley, Color32::WHITE);
    enabled && click.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// "Criar ou entrar em um servidor".
fn server(ui: &mut Ui, palette: &Palette, name: &mut String, code: &mut String, error: &str, focus: &mut bool, answer: &mut Answer) {
    if header(ui, palette, "Criar ou entrar em um servidor") {
        *answer = Answer::Close;
    }
    paragraph(
        ui,
        palette,
        "Um servidor junta pessoas com canais de texto e salas de voz. Não existe servidor central: as mensagens passam criptografadas e cada computador guarda o próprio histórico.",
    );
    // Here the paragraph's own margin is the room above the first section.
    ui.label(RichText::new("CRIAR UM SERVIDOR").color(palette.muted).size(12.0).family(theme::bold()));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let galley = ui.painter().layout_no_wrap("Criar".to_owned(), FontId::new(15.0, theme::semibold()), Color32::WHITE);
        let room = ui.available_width() - galley.size().x - 32.0 - 8.0;
        let field = ui.allocate_ui(Vec2::new(room, 38.0), |ui| text_field_hint(ui, palette, name, 40, "Nome do servidor")).inner;
        if std::mem::take(focus) {
            field.request_focus();
            select_all(ui, &field, name);
        }
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if button(ui, palette, "Criar", Tone::Primary).clicked() || entered {
            *answer = Answer::CreateServer(name.clone());
        }
    });
    section_title(ui, palette, "ENTRAR COM UM CONVITE");
    Frame::new().fill(palette.rail).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(10, 9)).show(ui, |ui| {
        ui.add(
            egui::TextEdit::multiline(code)
                .frame(Frame::new())
                .font(FontId::monospace(12.0))
                .text_color(palette.text)
                .hint_text(RichText::new("TELINHA-GRUPO.…").color(palette.muted).font(FontId::monospace(12.0)))
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
    });
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(INNER, 13.0), Sense::hover());
    if !error.is_empty() {
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, error, FontId::proportional(13.0), palette.danger_text);
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        if button(ui, palette, "Colar", Tone::Plain).clicked() {
            if let Ok(text) = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
                *code = text.trim().to_owned();
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if button(ui, palette, "Entrar", Tone::Primary).clicked() {
                *answer = Answer::JoinServer(code.clone());
            }
        });
    });
}

/// Selects the whole text of a field that just took the keyboard, as `input.select()` did.
fn select_all(ui: &Ui, field: &egui::Response, value: &str) {
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), field.id) {
        let range = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(value.chars().count()));
        state.cursor.set_char_range(Some(range));
        state.store(ui.ctx(), field.id);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Danger,
    /// The gray `.button`.
    Plain,
    /// `.button.ghost`: no background until the pointer is over it.
    Ghost,
}

/// A button as `.button` drew it: 34 pixels high, 16 of padding, white on gray, blue or red.
pub fn button(ui: &mut Ui, palette: &Palette, label: &str, tone: Tone) -> egui::Response {
    let font = FontId::new(15.0, theme::semibold());
    let ink = if tone == Tone::Ghost { palette.text } else { Color32::WHITE };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let size = Vec2::new(galley.size().x + 32.0, 34.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let enabled = ui.is_enabled();
    let hovered = response.hovered() && enabled;
    let lit = crate::motion::hover(ui.ctx(), response.id, hovered);
    let (rest, over) = match tone {
        Tone::Primary => (palette.accent, palette.accent_hover),
        Tone::Danger => (palette.danger, palette.danger_hover),
        Tone::Plain => (BUTTON, BUTTON_HOVER),
        Tone::Ghost => (Color32::TRANSPARENT, palette.hover),
    };
    let fill = crate::motion::mix(rest, over, lit);
    let fade = if enabled { 1.0 } else { 0.5 };
    if palette.glass && tone == Tone::Primary {
        // `.primary` on glass: the accent gradient, rounder, a little brighter under the mouse.
        let shape = egui::Shape::rect_filled(rect, CornerRadius::same(8), Color32::WHITE.gamma_multiply(fade));
        crate::glass::accent_fill(ui.painter(), shape);
        if lit > 0.0 {
            ui.painter().rect_filled(rect, CornerRadius::same(8), Color32::from_white_alpha((18.0 * lit) as u8));
        }
    } else {
        ui.painter().rect_filled(rect, CornerRadius::same(4), fill.gamma_multiply(fade));
    }
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, ink.gamma_multiply(fade));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A field of one line, as `.input`: dark, rounded, with a blue ring while it has the keyboard.
pub fn text_field(ui: &mut Ui, palette: &Palette, value: &mut String) -> egui::Response {
    text_field_hint(ui, palette, value, usize::MAX, "")
}

fn text_field_limited(ui: &mut Ui, palette: &Palette, value: &mut String, limit: usize) -> egui::Response {
    text_field_hint(ui, palette, value, limit, "")
}

fn text_field_hint(ui: &mut Ui, palette: &Palette, value: &mut String, limit: usize, hint: &str) -> egui::Response {
    let shown = Frame::new().fill(palette.rail).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(10, 9)).show(ui, |ui| {
        ui.add(
            egui::TextEdit::singleline(value)
                .frame(Frame::new())
                .font(FontId::proportional(15.0))
                .text_color(palette.text)
                .char_limit(limit)
                .hint_text(RichText::new(hint).color(palette.muted))
                .desired_width(ui.available_width()),
        )
    });
    if shown.inner.has_focus() {
        ui.painter().rect_stroke(shown.response.rect, CornerRadius::same(4), Stroke::new(2.0, palette.accent), egui::StrokeKind::Outside);
    }
    shown.inner
}
