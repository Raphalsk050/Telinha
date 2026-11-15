//! The conversation on screen: the messages, what they carry, and the box to write in.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use chrono::{Local, TimeZone};
use egui::{Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};
use regex::Regex;
use telinha_core::chats::{Embed, FileRef, ImageRef, Message};
use telinha_core::hub::{Conversation, FileStatus, Hub, Outgoing};

use crate::attach::{self, Attachment, MAX_INLINE_GIF_BYTES};
use crate::icons;
use crate::link_draft::{ForSend, LinkDraft, SEND_WAIT};
use crate::name_style::{Styles, name_label};
use crate::pictures::Pictures;
use crate::theme::{self, Palette};
use crate::widgets;

/// Messages from the same person this close together are drawn as one block.
const GROUP_WINDOW_MS: i64 = 5 * 60 * 1000;
const IMAGE_BOUNDS: Vec2 = Vec2::new(400.0, 300.0);
const EMBED_BOUNDS: Vec2 = Vec2::new(400.0, 225.0);
const CONTENT_LEFT: f32 = 72.0;
const CHAT_FONT: f32 = 15.5;

static LINK_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)\b(?:https?://|www\.)[^\s<>"]+"#).unwrap());

/// What the window knows about the conversation that is open, besides the messages.
pub struct Chrome {
    pub title: String,
    pub intro_icon: Option<icons::Icon>,
    pub intro_title: String,
    pub intro_text: String,
    pub placeholder: String,
    pub hint: String,
    /// The size of the text of the messages.
    pub font: f32,
    /// The space between blocks of messages.
    pub gap: f32,
    /// Messages without the picture of who wrote, to fit more on the screen.
    pub compact: bool,
}

/// What the person did in the chat that the rest of the window has to act on.
pub enum ChatAction {
    OpenImage { uri: String, bytes: Arc<[u8]> },
    /// Erase a message. `ask` is false when the person held Shift, to skip the question.
    Delete { message_id: String, ask: bool },
    Notice(String),
    /// Show the profile card of who wrote a message, next to where the person clicked.
    Profile { member_id: Option<String>, name: String, anchor: Rect },
}

/// A picture ready to draw: where the loaders find it, its bytes, and its size in pixels.
#[derive(Clone)]
struct Blob {
    uri: String,
    bytes: Arc<[u8]>,
    size: Vec2,
}

#[derive(Default)]
pub struct ChatView {
    conversation: Option<Conversation>,
    messages: Vec<Message>,
    draft: String,
    hint: String,
    blobs: HashMap<String, Option<Blob>>,
    attachment: Option<Attachment>,
    /// The picture of the attachment waiting to be sent, to show above the box.
    attachment_preview: Option<Blob>,
    to_bottom: bool,
    paste_was_down: bool,
    focus_input: bool,
    /// The large files on their way in or out, by the id of each.
    transfers: HashMap<String, Transfer>,
    /// The preview of the link in the draft.
    link: LinkDraft,
    /// A message held back for its preview to finish loading, and until when it waits.
    waiting_send: Option<f64>,
}

pub fn format_time(timestamp: i64) -> String {
    let Some(date) = Local.timestamp_millis_opt(timestamp).single() else {
        return String::new();
    };
    let time = date.format("%H:%M");
    let today = Local::now().date_naive();
    if date.date_naive() == today {
        format!("Hoje às {time}")
    } else if Some(date.date_naive()) == today.pred_opt() {
        format!("Ontem às {time}")
    } else {
        format!("{} {time}", date.format("%d/%m/%Y"))
    }
}

pub fn format_bytes(bytes: i64) -> String {
    let comma = |value: f64| format!("{value:.1}").replace('.', ",");
    match bytes {
        ..1024 => format!("{bytes} B"),
        ..1_048_576 => format!("{} KB", (bytes as f64 / 1024.0).round()),
        ..1_073_741_824 => format!("{} MB", comma(bytes as f64 / 1_048_576.0)),
        _ => format!("{} GB", comma(bytes as f64 / 1_073_741_824.0)),
    }
}

fn extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        _ => "webp",
    }
}

fn dimensions(bytes: &[u8]) -> Option<Vec2> {
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()?;
    Some(Vec2::new(width as f32, height as f32))
}

/// A link as it should be opened: without the punctuation that ends the sentence around it.
fn trim_link(raw: &str) -> &str {
    raw.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '\'', '"'])
}

fn link_target(text: &str) -> String {
    if text.to_lowercase().starts_with("www.") { format!("https://{text}") } else { text.to_owned() }
}

/// The first link written in a text, as it would be opened.
pub(crate) fn first_link(text: &str) -> Option<String> {
    LINK_PATTERN.find(text).map(|found| link_target(trim_link(found.as_str())))
}

/// Where a large file stands on its way in or out, for its card.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transfer {
    pub status: Option<FileStatus>,
    pub done: u64,
    pub total: u64,
    /// True while the file is leaving this computer, false while it is arriving.
    pub up: bool,
}

impl Transfer {
    /// How much of it went, from 0 to 1, while it is moving.
    fn fraction(&self) -> Option<f32> {
        let moving = self.total > 0 && self.done < self.total && !matches!(self.status, Some(FileStatus::Failed(_)));
        moving.then(|| self.done as f32 / self.total as f32)
    }
}

/// The line under the name of a file, and whether it tells of a failure.
fn transfer_text(file: &FileRef, transfer: Option<&Transfer>) -> (String, bool) {
    let size = format_bytes(file.size);
    let percent = |transfer: &Transfer| (transfer.done * 100).checked_div(transfer.total).unwrap_or(0);
    match transfer {
        Some(transfer) if transfer.up && transfer.done < transfer.total => (format!("{size} · enviando {}%", percent(transfer)), false),
        Some(Transfer { status: Some(FileStatus::Failed(reason)), .. }) if file.pending => {
            let why = match reason.as_str() {
                "nobody" => "ninguém que tem o arquivo está online",
                "offline" => "sem conexão com o servidor de contatos",
                "unreachable" => "não consegui abrir a conexão direta",
                "stalled" | "closed" => "a conexão caiu no meio",
                "corrupt" => "o arquivo chegou diferente do que foi enviado",
                "disk" => "não consegui gravar no disco",
                _ => "não chegou",
            };
            (format!("{size} · {why}"), true)
        }
        Some(transfer) if file.pending => match transfer.status {
            Some(FileStatus::Waiting) => (format!("{size} · na fila"), false),
            Some(FileStatus::Receiving) => (format!("{size} · recebendo {}%", percent(transfer)), false),
            _ => (format!("{size} · procurando quem tem o arquivo…"), false),
        },
        None if file.pending => (format!("{size} · ainda não chegou"), false),
        _ => (size, false),
    }
}

/// A GIF that came as a file because it was too large for a message, and is shown animated.
fn shows_animated(file: &FileRef) -> bool {
    file.mime == "image/gif" && !file.pending && file.size > 0 && file.size <= MAX_INLINE_GIF_BYTES
}

fn image_file_name(mime: &str, sent_at: i64) -> String {
    let stamp = Local.timestamp_millis_opt(sent_at).single().map_or_else(String::new, |date| date.format("%Y-%m-%d-%H-%M-%S").to_string());
    format!("telinha-{stamp}.{}", extension(mime))
}

impl ChatView {
    /// Shows another conversation, or none. The draft does not follow.
    pub fn open(&mut self, hub: &Hub, conversation: Option<Conversation>) {
        if self.conversation == conversation {
            return;
        }
        hub.set_viewing(conversation.as_ref());
        self.messages = conversation.as_ref().map_or_else(Vec::new, |conversation| hub.history(conversation));
        self.conversation = conversation;
        self.draft.clear();
        self.hint.clear();
        self.blobs.clear();
        self.attachment = None;
        self.attachment_preview = None;
        self.to_bottom = true;
        self.focus_input = true;
        self.link.clear();
        self.waiting_send = None;
    }

    /// The hub told where a large file stands. Kept for every conversation, since a file may be
    /// on its way while the person looks at another one.
    pub fn file_status(&mut self, file_id: &str, status: FileStatus) {
        self.transfers.entry(file_id.to_owned()).or_default().status = Some(status);
    }

    pub fn file_progress(&mut self, file_id: &str, done: u64, total: u64, up: bool) {
        let transfer = self.transfers.entry(file_id.to_owned()).or_default();
        (transfer.done, transfer.total, transfer.up) = (done, total, up);
        // A file that left whole has nothing more to show.
        if up && done >= total {
            self.transfers.remove(file_id);
        }
    }

    /// Shows a made-up link preview above the box. For the tests.
    pub fn link_for_test(&mut self, url: &str, site: &str, title: &str, description: &str) {
        self.link.show_for_test(url, site, title, description);
    }

    pub fn conversation(&self) -> Option<&Conversation> {
        self.conversation.as_ref()
    }

    pub fn received(&mut self, conversation: &Conversation, message: Message) {
        if self.conversation.as_ref() != Some(conversation) || self.messages.iter().any(|known| known.id == message.id) {
            return;
        }
        self.messages.push(message);
        self.messages.sort_by_key(|message| message.sent_at);
    }

    pub fn updated(&mut self, conversation: &Conversation, message: Message) {
        // A file that arrived whole has nothing more to tell about its way here.
        if let Some(file) = message.file.as_ref().filter(|file| !file.pending) {
            if self.transfers.get(&file.id).is_some_and(|transfer| !transfer.up) {
                self.transfers.remove(&file.id);
            }
        }
        if self.conversation.as_ref() != Some(conversation) {
            return;
        }
        if let Some(known) = self.messages.iter_mut().find(|known| known.id == message.id) {
            *known = message;
        }
    }

    pub fn removed(&mut self, conversation: &Conversation, message_id: &str) {
        if self.conversation.as_ref() == Some(conversation) {
            self.messages.retain(|message| message.id != message_id);
        }
    }

    /// Takes what will go with the next message, or says why it cannot.
    pub fn attach(&mut self, attachment: Result<Attachment, String>) {
        match attachment {
            Ok(attachment) => {
                self.attachment_preview = match &attachment {
                    Attachment::Image { image, .. } => {
                        let bytes: Arc<[u8]> = image.data.clone().into();
                        let uri = format!("bytes://pending/{}.{}", telinha_core::text::new_uuid(), extension(&image.mime));
                        Some(Blob { uri, bytes, size: Vec2::new(image.width as f32, image.height as f32) })
                    }
                    Attachment::File(_) => None,
                };
                self.attachment = Some(attachment);
                self.hint.clear();
                self.focus_input = true;
            }
            Err(error) => self.hint = format!("Não consegui anexar: {error}."),
        }
    }

    fn image_blob(&mut self, hub: &Hub, image: &ImageRef) -> Option<Blob> {
        let conversation = self.conversation.clone()?;
        self.blobs
            .entry(image.id.clone())
            .or_insert_with(|| {
                let (mime, data) = hub.chat_image(&conversation, &image.id)?;
                let size = Vec2::new(image.width.max(1) as f32, image.height.max(1) as f32);
                Some(Blob { uri: format!("bytes://chat/{}.{}", image.id, extension(&mime)), bytes: data.into(), size })
            })
            .clone()
    }

    fn gif_blob(&mut self, hub: &Hub, file: &FileRef) -> Option<Blob> {
        let conversation = self.conversation.clone()?;
        self.blobs
            .entry(file.id.clone())
            .or_insert_with(|| {
                let data = hub.chat_file(&conversation, &file.id)?;
                let size = dimensions(&data)?;
                Some(Blob { uri: format!("bytes://chat/{}.gif", file.id), bytes: data.into(), size })
            })
            .clone()
    }

    /// Sends what is in the box. A message whose link preview is still loading waits a moment
    /// for it, unless `now` is past the wait or `force` says to go without.
    fn send(&mut self, hub: &Arc<Hub>, context: &egui::Context, force: bool) {
        let Some(conversation) = self.conversation.clone() else {
            return;
        };
        let text = self.draft.trim().to_owned();
        if text.is_empty() && self.attachment.is_none() {
            self.waiting_send = None;
            return;
        }
        let mut outgoing = Outgoing { text, ..Outgoing::default() };
        if !outgoing.text.is_empty() {
            match self.link.for_send(context, &self.draft) {
                ForSend::Wait if !force => {
                    let now = context.input(|input| input.time);
                    self.waiting_send.get_or_insert(now + SEND_WAIT);
                    context.request_repaint_after(std::time::Duration::from_millis(100));
                    return;
                }
                ForSend::Embed(embed) => outgoing.embed = Some(embed),
                ForSend::Wait | ForSend::Plain => {}
            }
        }
        self.waiting_send = None;
        match self.attachment.take() {
            Some(Attachment::Image { image, .. }) => outgoing.image = Some(image),
            Some(Attachment::File(file)) => outgoing.file = Some(file),
            None => {}
        }
        self.attachment_preview = None;
        match hub.send_chat(&conversation, outgoing) {
            Ok(Some(message)) => {
                self.draft.clear();
                self.hint.clear();
                self.link.clear();
                self.received(&conversation, message);
                self.to_bottom = true;
            }
            Ok(None) => {}
            Err(error) => self.hint = format!("Não consegui enviar: {error}."),
        }
    }

    pub fn show(&mut self, ui: &mut Ui, hub: &Arc<Hub>, pictures: &mut Pictures, styles: &Styles, palette: &Palette, chrome: &Chrome) -> Option<ChatAction> {
        let mut action = None;
        self.take_dropped(ui);
        self.link.tend(ui.ctx(), &self.draft);
        let pending = if self.attachment.is_some() { 64.0 } else { 0.0 };
        let composer_height = 44.0 + 28.0 + pending + self.link.height();
        let list_height = (ui.available_height() - composer_height).max(0.0);
        ui.allocate_ui(Vec2::new(ui.available_width(), list_height), |ui| {
            let to_bottom = std::mem::take(&mut self.to_bottom);
            let area = egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true);
            area.show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.intro(ui, palette, chrome);
                let messages = std::mem::take(&mut self.messages);
                for (index, message) in messages.iter().enumerate() {
                    let previous = index.checked_sub(1).map(|index| &messages[index]);
                    let first = previous.is_none_or(|previous| !same_block(previous, message));
                    if let Some(found) = self.message(ui, hub, pictures, styles, palette, chrome, message, first) {
                        action = Some(found);
                    }
                }
                self.messages = messages;
                ui.add_space(20.0);
                if to_bottom {
                    ui.scroll_to_cursor(Some(Align::BOTTOM));
                }
            });
        });
        self.composer(ui, hub, palette, chrome);
        action
    }

    /// Files dropped on the window become the attachment of the next message.
    fn take_dropped(&mut self, ui: &Ui) {
        let dropped = ui.input(|input| input.raw.dropped_files.first().cloned());
        let Some(file) = dropped else { return };
        self.attach(attach::from_path(file.path()));
    }

    fn intro(&self, ui: &mut Ui, palette: &Palette, chrome: &Chrome) {
        ui.add_space(24.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.vertical(|ui| {
                let (circle, _) = ui.allocate_exact_size(Vec2::splat(68.0), Sense::hover());
                ui.painter().circle_filled(circle.center(), 34.0, palette.raised);
                if let Some(icon) = chrome.intro_icon {
                    icon.image(40.0, palette.strong).paint_at(ui, Rect::from_center_size(circle.center(), Vec2::splat(40.0)));
                } else {
                    widgets::paint_avatar(ui, circle, &chrome.title, None);
                }
                ui.add_space(8.0);
                ui.label(RichText::new(&chrome.intro_title).color(palette.strong).size(28.0).family(theme::bold()));
                ui.add_space(2.0);
                ui.set_max_width((ui.available_width() - 32.0).min(760.0));
                ui.label(RichText::new(&chrome.intro_text).color(palette.muted).size(14.0));
            });
        });
        ui.add_space(8.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn message(&mut self, ui: &mut Ui, hub: &Arc<Hub>, pictures: &mut Pictures, styles: &Styles, palette: &Palette, chrome: &Chrome, message: &Message, first: bool) -> Option<ChatAction> {
        let mut action = None;
        if first {
            ui.add_space(chrome.gap);
        }
        let member = if message.mine { Some(hub.member_id()) } else { message.member_id.clone() };
        let row = ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if chrome.compact {
                ui.add_space(16.0);
            } else if first {
                ui.add_space(16.0);
                let face = widgets::avatar(ui, 40.0, &message.author, pictures.avatar(hub, member.as_deref())).interact(Sense::click());
                let anchor = face.rect;
                if face.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    action = Some(ChatAction::Profile { member_id: member.clone(), name: message.author.clone(), anchor });
                }
                ui.add_space(16.0);
            } else {
                ui.add_space(CONTENT_LEFT);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.set_max_width(ui.available_width() - 48.0);
                if first {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let name = name_label(ui, &message.author, FontId::new(15.0, theme::semibold()), palette.strong, styles.of(member.as_deref()), false);
                        let anchor = name.rect;
                        if name.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            action = Some(ChatAction::Profile { member_id: member.clone(), name: message.author.clone(), anchor });
                        }
                        ui.label(RichText::new(format_time(message.sent_at)).color(palette.muted).size(12.0));
                    });
                }
                if !message.text.is_empty() {
                    paint_text(ui, palette, &message.text, chrome.font);
                }
                if let Some(embed) = &message.embed {
                    if let Some(found) = self.embed(ui, hub, palette, message, embed) {
                        action = Some(found);
                    }
                }
                if let Some(image) = &message.image {
                    let blob = self.image_blob(hub, image);
                    if let Some(found) = picture(ui, palette, blob, IMAGE_BOUNDS, &image_file_name(&image.mime, message.sent_at)) {
                        action = Some(found);
                    }
                }
                if let Some(file) = &message.file {
                    let animated = if shows_animated(file) { self.gif_blob(hub, file) } else { None };
                    let found = match animated {
                        Some(blob) => picture(ui, palette, Some(blob), IMAGE_BOUNDS, &file.name),
                        None => self.file_card(ui, hub, palette, file),
                    };
                    if let Some(found) = found {
                        action = Some(found);
                    }
                }
                if message.mine && message.delivered == Some(false) {
                    ui.label(RichText::new("(não enviada, sem conexão com o servidor)").color(palette.danger_text).size(12.0));
                }
            });
        });

        // The tools of a message show on its right while the cursor is over it.
        let rect = Rect::from_min_max(egui::pos2(ui.max_rect().left(), row.response.rect.top()), egui::pos2(ui.max_rect().right(), row.response.rect.bottom()));
        let hovered = ui.rect_contains_pointer(rect);
        if hovered && message.mine {
            let tool = Rect::from_min_size(egui::pos2(rect.right() - 44.0, rect.top() - 2.0), Vec2::splat(28.0));
            let response = ui.interact(tool, ui.scope_id().with(("delete", &message.id)), Sense::click());
            ui.painter().rect_filled(tool, CornerRadius::same(4), if response.hovered() { palette.danger } else { palette.panel });
            ui.painter().rect_stroke(tool, CornerRadius::same(4), Stroke::new(1.0, palette.line), egui::StrokeKind::Inside);
            let ink = if response.hovered() { Color32::WHITE } else { palette.soft };
            icons::DELETE.image(16.0, ink).paint_at(ui, Rect::from_center_size(tool.center(), Vec2::splat(16.0)));
            if response.on_hover_text("Apagar mensagem (Shift apaga sem perguntar)").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                let ask = !ui.input(|input| input.modifiers.shift);
                action = Some(ChatAction::Delete { message_id: message.id.clone(), ask });
            }
        }
        let menu = ui.interact(rect, ui.scope_id().with(("menu", &message.id)), Sense::hover());
        // The text cannot be selected here as it could there, so the menu also copies it.
        if !message.text.is_empty() || message.mine {
            crate::app::context_menu(&menu, palette).show(|ui| {
                if !message.text.is_empty() && crate::app::menu_item(ui, palette, "Copiar texto", false) {
                    ui.ctx().copy_text(message.text.clone());
                }
                if message.mine {
                    if !message.text.is_empty() {
                        crate::app::menu_separator(ui, palette);
                    }
                    if crate::app::menu_item(ui, palette, "Apagar mensagem", true) {
                        let ask = !ui.input(|input| input.modifiers.shift);
                        action = Some(ChatAction::Delete { message_id: message.id.clone(), ask });
                    }
                }
            });
        }
        action
    }

    fn embed(&mut self, ui: &mut Ui, hub: &Arc<Hub>, palette: &Palette, message: &Message, embed: &Embed) -> Option<ChatAction> {
        let mut action = None;
        ui.add_space(4.0);
        let frame = Frame::new().fill(palette.panel).corner_radius(CornerRadius::same(6)).inner_margin(Margin { left: 16, right: 14, top: 10, bottom: 12 });
        let inner = frame.show(ui, |ui| {
            ui.set_max_width(432.0);
            ui.spacing_mut().item_spacing.y = 4.0;
            if !embed.site_name.is_empty() {
                ui.label(RichText::new(&embed.site_name).color(palette.muted).size(12.0));
            }
            if !embed.title.is_empty() {
                let title = RichText::new(&embed.title).color(palette.link).size(15.0).family(theme::semibold());
                if ui.add(egui::Label::new(title).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(&embed.url));
                }
            }
            if !embed.description.is_empty() {
                ui.label(RichText::new(&embed.description).color(palette.text).size(14.0));
            }
            if let Some(image) = &embed.image {
                let blob = self.image_blob(hub, image);
                if let Some(found) = picture(ui, palette, blob, EMBED_BOUNDS, &image_file_name(&image.mime, message.sent_at)) {
                    action = Some(found);
                }
            }
        });
        let bar = Rect::from_min_max(inner.response.rect.left_top(), inner.response.rect.left_bottom() + Vec2::new(4.0, 0.0));
        ui.painter().rect_filled(bar, CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 }, palette.accent);
        action
    }

    fn file_card(&mut self, ui: &mut Ui, hub: &Arc<Hub>, palette: &Palette, file: &FileRef) -> Option<ChatAction> {
        let mut action = None;
        ui.add_space(4.0);
        Frame::new().fill(palette.panel).stroke(Stroke::new(1.0, palette.line)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(390.0);
            ui.horizontal(|ui| {
                ui.add(icons::FILE.image(34.0, palette.accent));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.set_max_width(290.0);
                    ui.add(egui::Label::new(RichText::new(&file.name).color(palette.link).size(15.0)).truncate());
                    let transfer = self.transfers.get(&file.id);
                    let (status, failed) = transfer_text(file, transfer);
                    ui.label(RichText::new(status).color(if failed { palette.danger_text } else { palette.muted }).size(12.0));
                    if let Some(part) = transfer.and_then(Transfer::fraction) {
                        let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width().min(290.0), 4.0), Sense::hover());
                        ui.painter().rect_filled(bar, CornerRadius::same(2), palette.raised);
                        ui.painter().rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * part, bar.height())), CornerRadius::same(2), palette.accent);
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let transfer = self.transfers.get(&file.id);
                    if file.pending {
                        // A file that is not on its way can be asked for again.
                        let idle = transfer.is_none_or(|transfer| matches!(transfer.status, Some(FileStatus::Failed(_))));
                        if idle && widgets::icon_button(ui, icons::DOWNLOAD, 30.0, palette.soft, palette.selected, "Baixar o arquivo").clicked() {
                            if let Some(conversation) = &self.conversation {
                                self.transfers.remove(&file.id);
                                hub.start_download(conversation, &file.id);
                            }
                        }
                    } else if widgets::icon_button(ui, icons::DOWNLOAD, 30.0, palette.soft, palette.selected, "Salvar arquivo").clicked() {
                        // Copied piece by piece: a large file never has to fit in memory.
                        let saved = match (attach::pick_save_path(&file.name), &self.conversation) {
                            (Some(path), Some(conversation)) => hub.copy_chat_file(conversation, &file.id, &path).map(|()| true),
                            _ => Ok(false),
                        };
                        action = Some(ChatAction::Notice(match saved {
                            Ok(true) => format!("Arquivo salvo: {}", file.name),
                            Ok(false) => String::new(),
                            Err(error) => format!("Não consegui salvar: {error}."),
                        }));
                    }
                });
            });
        });
        action.filter(|action| !matches!(action, ChatAction::Notice(text) if text.is_empty()))
    }

    fn composer(&mut self, ui: &mut Ui, hub: &Arc<Hub>, palette: &Palette, chrome: &Chrome) {
        let mut send = false;
        Frame::new().inner_margin(Margin { left: 16, right: 16, top: 0, bottom: 0 }).show(ui, |ui| {
            self.pending_strip(ui, palette);
            self.link.show(ui, palette);
            Frame::new().fill(palette.raised).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(12, 7)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let attach = widgets::icon_button(ui, icons::ATTACH, 28.0, palette.soft, palette.selected, "Anexar um arquivo");
                    let plus = Rect::from_center_size(attach.rect.center(), Vec2::splat(12.0));
                    ui.painter().line_segment([plus.center_top(), plus.center_bottom()], Stroke::new(2.0, palette.raised));
                    ui.painter().line_segment([plus.left_center(), plus.right_center()], Stroke::new(2.0, palette.raised));
                    if attach.clicked() {
                        if let Some(path) = attach::pick_file() {
                            self.attach(attach::from_path(&path));
                        }
                    }
                    let width = ui.available_width() - 40.0;
                    let edit = egui::TextEdit::multiline(&mut self.draft)
                        .hint_text(RichText::new(&chrome.placeholder).color(palette.muted))
                        .font(FontId::proportional(CHAT_FONT))
                        .text_color(palette.text)
                        .frame(Frame::new())
                        .desired_rows(1)
                        .desired_width(width)
                        .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter));
                    let response = ui.add(edit);
                    if std::mem::take(&mut self.focus_input) {
                        response.request_focus();
                    }
                    if response.has_focus() {
                        if ui.input(|input| input.key_pressed(egui::Key::Enter) && !input.modifiers.shift) {
                            send = true;
                        }
                        // A picture or a file on the clipboard, pasted with Ctrl+V.
                        let down = attach::paste_keys_down();
                        if down && !self.paste_was_down {
                            if let Some(pasted) = attach::from_clipboard() {
                                self.attach(pasted);
                            }
                        }
                        self.paste_was_down = down;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let empty = self.draft.trim().is_empty() && self.attachment.is_none();
                        let ink = if empty { palette.muted } else { palette.accent_text };
                        if widgets::icon_button(ui, icons::SEND, 28.0, ink, palette.selected, "Enviar").clicked() {
                            send = true;
                        }
                    });
                });
            });
            ui.add_space(4.0);
            let hint = if self.hint.is_empty() { &chrome.hint } else { &self.hint };
            ui.label(RichText::new(hint).color(palette.muted).size(12.0));
        });
        if send {
            self.send(hub, ui.ctx(), false);
        } else if let Some(deadline) = self.waiting_send {
            // The message that was waiting goes as soon as its preview is ready or gives up.
            let late = ui.input(|input| input.time) >= deadline;
            if late || !self.link.loading() {
                self.send(hub, ui.ctx(), late);
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
    }

    /// What is attached to the next message, above the box, with a way to take it off.
    fn pending_strip(&mut self, ui: &mut Ui, palette: &Palette) {
        let Some(attachment) = &self.attachment else { return };
        let mut remove = false;
        Frame::new().fill(palette.panel).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                match &self.attachment_preview {
                    Some(blob) => {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                        egui::Image::from_bytes(blob.uri.clone(), egui::load::Bytes::Shared(Arc::clone(&blob.bytes))).corner_radius(CornerRadius::same(4)).paint_at(ui, rect);
                    }
                    None => {
                        ui.add(icons::FILE.image(34.0, palette.accent));
                    }
                }
                let details = match attachment {
                    Attachment::Image { image, .. } => format!("{}×{} · {}", image.width, image.height, format_bytes(attachment.size() as i64)),
                    Attachment::File(_) => format_bytes(attachment.size() as i64),
                };
                ui.add(egui::Label::new(RichText::new(format!("{} · {details}", attachment.name())).color(palette.text).size(14.0)).truncate());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    remove = widgets::icon_button(ui, icons::CLOSE, 28.0, palette.soft, palette.selected, "Tirar o anexo").clicked();
                });
            });
        });
        ui.add_space(4.0);
        if remove {
            self.attachment = None;
            self.attachment_preview = None;
        }
    }
}

fn same_block(previous: &Message, message: &Message) -> bool {
    previous.mine == message.mine
        && previous.author == message.author
        && previous.member_id == message.member_id
        && message.sent_at - previous.sent_at < GROUP_WINDOW_MS
}

/// A picture of a message, shrunk to fit the bounds. A click opens it, and the right button
/// offers to copy or save it.
fn picture(ui: &mut Ui, palette: &Palette, blob: Option<Blob>, bounds: Vec2, file_name: &str) -> Option<ChatAction> {
    ui.add_space(4.0);
    let Some(blob) = blob else {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(240.0, 120.0), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(8), palette.rail);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "imagem indisponível", FontId::proportional(12.0), palette.muted);
        return None;
    };
    let scale = (bounds.x / blob.size.x).min(bounds.y / blob.size.y).min(1.0);
    let shown = (blob.size * scale).max(Vec2::splat(60.0));
    let (rect, response) = ui.allocate_exact_size(shown, Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::same(8), palette.rail);
    egui::Image::from_bytes(blob.uri.clone(), egui::load::Bytes::Shared(Arc::clone(&blob.bytes))).corner_radius(CornerRadius::same(8)).paint_at(ui, rect);

    let mut action = None;
    crate::app::context_menu(&response, palette).show(|ui| {
        if crate::app::menu_item(ui, palette, "Copiar imagem", false) {
            action = Some(ChatAction::Notice(match attach::copy_image(&blob.bytes) {
                Ok(()) => "Imagem copiada.".to_owned(),
                Err(error) => format!("Não consegui copiar: {error}."),
            }));
        }
        if crate::app::menu_item(ui, palette, "Salvar imagem", false) {
            match attach::save_as(file_name, &blob.bytes) {
                Ok(true) => action = Some(ChatAction::Notice("Imagem salva.".to_owned())),
                Ok(false) => {}
                Err(error) => action = Some(ChatAction::Notice(format!("Não consegui salvar: {error}."))),
            }
        }
    });
    if response.on_hover_cursor(egui::CursorIcon::ZoomIn).clicked() {
        action = Some(ChatAction::OpenImage { uri: blob.uri, bytes: blob.bytes });
    }
    action
}

/// The text of a message, with links that open the browser.
fn paint_text(ui: &mut Ui, palette: &Palette, text: &str, size: f32) {
    let font = FontId::proportional(size);
    for line in text.split('\n') {
        if !LINK_PATTERN.is_match(line) {
            ui.label(RichText::new(line).color(palette.text).font(font.clone()));
            continue;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let mut rest = 0;
            for found in LINK_PATTERN.find_iter(line) {
                let link = trim_link(found.as_str());
                let end = found.start() + link.len();
                if found.start() > rest {
                    ui.label(RichText::new(&line[rest..found.start()]).color(palette.text).font(font.clone()));
                }
                let label = egui::Label::new(RichText::new(link).color(palette.link).font(font.clone())).sense(Sense::click());
                let response = ui.add(label).on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.hovered() {
                    let y = response.rect.bottom() - 1.0;
                    ui.painter().hline(response.rect.x_range(), y, Stroke::new(1.0, palette.link));
                }
                if response.clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(link_target(link)));
                }
                rest = end;
            }
            if rest < line.len() {
                ui.label(RichText::new(&line[rest..]).color(palette.text).font(font.clone()));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_written_with_a_comma_like_before() {
        assert_eq!(format_bytes(900), "900 B");
        assert_eq!(format_bytes(2048), "2 KB");
        assert_eq!(format_bytes(5_400_000), "5,1 MB");
        assert_eq!(format_bytes(3 * 1_073_741_824), "3,0 GB");
    }

    #[test]
    fn a_link_loses_the_punctuation_of_the_sentence_around_it() {
        assert_eq!(trim_link("https://example.com/a)."), "https://example.com/a");
        assert_eq!(link_target("www.example.com"), "https://www.example.com");
        assert_eq!(link_target("http://example.com"), "http://example.com");
    }
}
