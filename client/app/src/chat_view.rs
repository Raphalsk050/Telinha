//! The conversation on screen: the messages, what they carry, and the box to write in.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use chrono::{Local, TimeZone};
use egui::{Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};
use regex::Regex;
use telinha_core::chats::{Embed, FileRef, ImageRef, MAX_TEXT_LENGTH, Message};
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
/// The least time between two calls for attention, in seconds.
const NUDGE_PAUSE: f64 = 10.0;
const CHAT_FONT: f32 = 15.5;
const COMPOSER_MAX: f32 = 200.0;

static LINK_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)\b(?:https?://|www\.)[^\s<>"]+"#).unwrap());

/// What the window knows about the conversation that is open, besides the messages.
#[derive(Clone)]
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
    /// Open the thread that hangs from a message, beside the conversation.
    OpenThread(String),
}

/// The answers in the thread of a message, for the bar under it.
#[derive(Default)]
struct ThreadSummary {
    count: usize,
    last_at: i64,
    /// Who answered, the first three, by member and name.
    faces: Vec<(Option<String>, String)>,
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
    /// The video playing here, by the key of its message and link, and where its player goes
    /// this frame, when all of it is in sight.
    playing: Option<(String, crate::media::Video)>,
    player_rect: Option<Rect>,
    /// The moving pictures of the GIF links shown.
    animated: crate::media::AnimatedLinks,
    /// When the last call for attention went out, to space them.
    last_nudge: Option<f64>,
    focus_input: bool,
    /// The large files on their way in or out, by the id of each.
    transfers: HashMap<String, Transfer>,
    /// The preview of the link in the draft.
    link: LinkDraft,
    /// A message held back for its preview to finish loading, and until when it waits.
    waiting_send: Option<f64>,
    /// How tall the box at the bottom was drawn last time; it grows with the draft.
    composer_height: f32,
    /// Whether the message box had the keyboard on the last frame, for its ring on glass.
    focused: bool,
    /// Whether the last message was in sight on the last frame.
    at_end: bool,
    /// Where the tests keep the conversation scrolled.
    scroll_test: Option<f32>,
    /// Where the message box was on the last frame.
    box_rect: Option<Rect>,
    /// How tall the text of the draft was last time, which the box follows up to its limit.
    draft_height: f32,
    /// When this view shows a thread, as Slack: the id of the message it hangs from.
    thread: Option<String>,
    /// A message to bring into sight and light up for a moment, and when it was reached.
    jump: Option<(String, Option<f64>)>,
    /// The message whose tools showed on the last frame, and where the tools were: the pointer
    /// on them keeps them, even over the message above.
    tools_for: Option<(String, Rect)>,
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
        self.playing = None;
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

    /// Shows the thread that hangs from `root` in `conversation`.
    pub fn open_thread(&mut self, hub: &Hub, conversation: Conversation, root: &str) {
        self.open(hub, Some(conversation));
        self.thread = Some(root.to_owned());
    }

    /// The message the thread shown here hangs from, when this view shows a thread.
    pub fn thread_root(&self) -> Option<&Message> {
        let root = self.thread.as_deref()?;
        self.messages.iter().find(|message| message.id == root)
    }

    /// The pinned messages of the conversation, the newest first.
    pub fn pinned(&self) -> Vec<Message> {
        self.messages.iter().rev().filter(|message| message.is_pinned()).cloned().collect()
    }

    pub fn message_by_id(&self, message_id: &str) -> Option<&Message> {
        self.messages.iter().find(|message| message.id == message_id)
    }

    /// Brings a message into sight and lights it up for a moment.
    pub fn jump_to(&mut self, message_id: &str) {
        self.jump = Some((message_id.to_owned(), None));
        self.to_bottom = false;
        self.scroll_test = None;
    }

    /// Stops the video playing here.
    pub fn stop_video(&mut self) {
        self.playing = None;
    }

    /// Unpins or pins a message here and for everyone in the conversation.
    fn set_pinned(&mut self, hub: &Hub, message_id: &str, pinned: bool) {
        let Some(conversation) = self.conversation.clone() else { return };
        if hub.pin_message(&conversation, message_id, pinned).is_none() {
            self.hint = "Não consegui fixar essa mensagem.".to_owned();
        }
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
    pub fn draft_for_test(&mut self, text: &str) {
        self.draft = text.to_owned();
    }

    pub fn composer_height_for_test(&self) -> f32 {
        self.composer_height
    }

    /// Scrolls the conversation this far from its top on the next frame. For the tests.
    pub fn scroll_for_test(&mut self, offset: f32) {
        self.scroll_test = Some(offset);
        self.to_bottom = false;
    }

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
        let mut text = self.draft.trim().to_owned();
        if text.is_empty() && self.attachment.is_none() {
            self.waiting_send = None;
            return;
        }
        // Longer than a message carries: as in Discord, the whole text goes as a .txt file
        // instead of being cut at the limit.
        if too_long(&text) {
            self.waiting_send = None;
            if self.attachment.is_some() {
                self.hint = format!("O texto passa de {MAX_TEXT_LENGTH} caracteres. Tire o anexo para enviá-lo como arquivo.");
                return;
            }
            match attach::from_bytes("mensagem.txt", text.clone().into_bytes()) {
                Ok(file) => self.attachment = Some(file),
                Err(error) => {
                    self.hint = format!("Não consegui enviar: {error}.");
                    return;
                }
            }
            text.clear();
        }
        let mut outgoing = Outgoing { text, thread: self.thread.clone(), ..Outgoing::default() };
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

    /// A call for attention, as a line of the conversation: the bell where the avatars are, who
    /// called and when. The bell rings for a moment after it arrives.
    fn nudge_line(ui: &mut Ui, palette: &Palette, chrome: &Chrome, message: &Message) {
        ui.add_space(chrome.gap.max(8.0));
        let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
        let bell_center = egui::pos2(row.left() + 16.0 + 20.0, row.center().y);
        let age = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_millis() as i64) - message.sent_at) as f32 / 1000.0;
        let angle = if (0.0..1.6).contains(&age) {
            ui.ctx().request_repaint();
            (age * 28.0).sin() * 0.45 * (1.0 - age / 1.6)
        } else {
            0.0
        };
        let bell = Rect::from_center_size(bell_center, Vec2::splat(18.0));
        icons::BELL.image(18.0, palette.accent_text).rotate(angle, Vec2::new(0.5, 0.15)).paint_at(ui, bell);
        let who = if message.mine { "Você".to_owned() } else { message.author.clone() };
        let font = FontId::proportional(chrome.font.max(14.0));
        let mut job = egui::text::LayoutJob::default();
        job.append(&who, 0.0, egui::TextFormat { font_id: FontId::new(font.size, theme::semibold()), color: palette.strong, ..Default::default() });
        job.append(" chamou a atenção!", 0.0, egui::TextFormat { font_id: font.clone(), color: palette.text, ..Default::default() });
        job.append(&format!("   {}", format_time(message.sent_at)), 0.0, egui::TextFormat { font_id: FontId::proportional(12.0), color: palette.muted, ..Default::default() });
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(row.left() + CONTENT_LEFT, row.center().y - galley.size().y / 2.0), galley, palette.text);
    }

    /// Sends a call for attention, at most one every ten seconds.
    fn send_nudge(&mut self, hub: &Arc<Hub>, context: &egui::Context) {
        let Some(conversation) = self.conversation.clone() else { return };
        let now = context.input(|input| input.time);
        if self.last_nudge.is_some_and(|last| now - last < NUDGE_PAUSE) {
            self.hint = "Espere um pouco para chamar a atenção de novo.".to_owned();
            return;
        }
        self.last_nudge = Some(now);
        let outgoing = Outgoing { text: "chamou a atenção!".to_owned(), nudge: true, ..Outgoing::default() };
        match hub.send_chat(&conversation, outgoing) {
            Ok(Some(message)) => {
                self.received(&conversation, message);
                self.to_bottom = true;
            }
            Ok(None) => {}
            Err(error) => self.hint = format!("Não consegui chamar a atenção: {error}."),
        }
    }

    /// A video of a link, as Discord plays them: its picture with a play button, and in its place
    /// the player once it was clicked. The player is a window of its own laid over the place kept
    /// here, so only the place is drawn.
    fn video_block(&mut self, ui: &mut Ui, palette: &Palette, key: &str, video: &crate::media::Video, still: Option<Blob>) {
        ui.add_space(4.0);
        let size = if video.tall { Vec2::new(243.0, 432.0) } else { Vec2::new(432.0, 243.0) };
        let size = size * (ui.available_width().max(160.0) / size.x).min(1.0);
        let round = CornerRadius::same(if palette.glass { 14 } else { 8 });
        let playing = self.playing.as_ref().is_some_and(|(playing, _)| playing == key);
        let (rect, response) = ui.allocate_exact_size(size, if playing { Sense::hover() } else { Sense::click() });
        if palette.glass {
            ui.painter().add(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(77) }.as_shape(rect, round));
        }
        ui.painter().rect_filled(rect, round, Color32::BLACK);
        if playing {
            // The player shows only while all of it is in sight and the message box is not over it.
            let seen = ui.clip_rect().contains_rect(rect) && !self.box_rect.is_some_and(|boxed| boxed.intersects(rect));
            if seen {
                self.player_rect = Some(rect);
            } else {
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, "Tocando…", FontId::proportional(14.0), palette.muted);
            }
            // The way to stop it, under the video, outside the window of the player.
            ui.add_space(6.0);
            let stop = ui.add(egui::Label::new(RichText::new("Fechar o vídeo").color(palette.link).size(13.0)).sense(Sense::click()));
            if stop.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.playing = None;
            }
            return;
        }
        if let Some(still) = still {
            // The picture fills the place, cut to its shape, with the round corners of the place.
            let scale = (size.x / still.size.x).max(size.y / still.size.y);
            let shown = still.size * scale;
            let uv = Rect::from_center_size(egui::pos2(0.5, 0.5), Vec2::new(size.x / shown.x, size.y / shown.y));
            egui::Image::from_bytes(still.uri.clone(), egui::load::Bytes::Shared(Arc::clone(&still.bytes))).uv(uv).corner_radius(round).paint_at(ui, rect);
        }
        let lit = crate::motion::hover(ui.ctx(), response.id, response.hovered());
        let radius = crate::motion::lerp(28.0, 31.0, lit);
        ui.painter().circle_filled(rect.center(), radius, crate::motion::mix(Color32::from_black_alpha(160), palette.accent, lit));
        ui.painter().circle_stroke(rect.center(), radius, Stroke::new(1.0, Color32::from_white_alpha(60)));
        let tip = rect.center() + Vec2::new(3.0, 0.0);
        let triangle = vec![tip + Vec2::new(-8.0, -11.0), tip + Vec2::new(11.0, 0.0), tip + Vec2::new(-8.0, 11.0)];
        ui.painter().add(egui::Shape::convex_polygon(triangle, Color32::WHITE, Stroke::NONE));
        if response.on_hover_text("Assistir aqui").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.playing = Some((key.to_owned(), video.clone()));
        }
    }

    /// The video playing in the conversation, by its key, and where its player goes this frame.
    pub fn playing(&self) -> Option<(&str, &crate::media::Video)> {
        self.playing.as_ref().map(|(key, video)| (key.as_str(), video))
    }

    pub fn player_rect(&self) -> Option<Rect> {
        self.player_rect
    }


    pub fn show(&mut self, ui: &mut Ui, hub: &Arc<Hub>, pictures: &mut Pictures, styles: &Styles, palette: &Palette, chrome: &Chrome) -> Option<ChatAction> {
        let mut action = None;
        self.player_rect = None;
        self.take_dropped(ui);
        self.link.tend(ui.ctx(), &self.draft);
        let pending = if self.attachment.is_some() { 64.0 } else { 0.0 };
        let composer_height = self.composer_height.max(44.0 + 28.0 + pending + self.link.height());
        // On glass the message box floats over the conversation, which passes behind it.
        let floating = palette.glass;
        let list_top = ui.cursor().min.y;
        // The room left beside the members, not the whole panel under the header.
        let given = ui.available_rect_before_wrap();
        let list_height = if floating { ui.available_height() } else { (ui.available_height() - composer_height).max(0.0) };
        let list = ui.allocate_ui(Vec2::new(ui.available_width(), list_height), |ui| {
            let to_bottom = std::mem::take(&mut self.to_bottom);
            let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true);
            // The wheel over a long draft scrolls the draft, not the conversation behind the box.
            let over_draft = self.draft_height > COMPOSER_MAX && ui.ctx().pointer_hover_pos().zip(self.box_rect).is_some_and(|(pointer, rect)| rect.contains(pointer));
            if over_draft {
                area = area.scroll_source(egui::scroll_area::ScrollSource { mouse_wheel: false, ..Default::default() });
            }
            if let Some(offset) = self.scroll_test {
                area = area.vertical_scroll_offset(offset);
            }
            let shown = area.show(ui, |ui| {
                ui.set_width(ui.available_width());
                let messages = std::mem::take(&mut self.messages);
                // The answers in threads stay in their threads; an answer whose message is gone
                // shows in the conversation.
                let ids: HashSet<&str> = messages.iter().map(|message| message.id.as_str()).collect();
                let threaded = |message: &Message| message.thread_id().is_some_and(|root| ids.contains(root));
                let mut threads: HashMap<&str, ThreadSummary> = HashMap::new();
                for message in messages.iter().filter(|message| threaded(message)) {
                    let summary = threads.entry(message.thread_id().unwrap_or_default()).or_default();
                    summary.count += 1;
                    summary.last_at = summary.last_at.max(message.sent_at);
                    let member = if message.mine { Some(hub.member_id()) } else { message.member_id.clone() };
                    if summary.faces.len() < 3 && !summary.faces.iter().any(|(known, name)| known == &member && name == &message.author) {
                        summary.faces.push((member, message.author.clone()));
                    }
                }
                let root = self.thread.clone();
                let shown: Vec<&Message> = match root.as_deref() {
                    None => messages.iter().filter(|message| !threaded(message)).collect(),
                    Some(root) => messages.iter().filter(|message| message.id == root || message.thread_id() == Some(root)).collect(),
                };
                let pointer = ui.ctx().pointer_hover_pos();
                let sticky = self.tools_for.take().filter(|(_, tools)| pointer.is_some_and(|pointer| tools.contains(pointer))).map(|(id, _)| id);
                match root.as_deref() {
                    None => self.intro(ui, palette, chrome),
                    Some(_) => ui.add_space(8.0),
                }
                for (index, message) in shown.iter().enumerate() {
                    let previous = index.checked_sub(1).map(|index| shown[index]);
                    if message.is_nudge() {
                        Self::nudge_line(ui, palette, chrome, message);
                        continue;
                    }
                    let after_root = previous.is_some_and(|previous| Some(previous.id.as_str()) == root.as_deref());
                    let first = after_root || previous.is_none_or(|previous| previous.is_nudge() || !same_block(previous, message));
                    let summary = if root.is_none() { threads.get(message.id.as_str()) } else { None };
                    if let Some(found) = self.message(ui, hub, pictures, styles, palette, chrome, message, first, summary, sticky.as_deref()) {
                        action = Some(found);
                    }
                    if Some(message.id.as_str()) == root.as_deref() {
                        thread_divider(ui, palette, shown.len() - 1);
                    }
                }
                self.messages = messages;
                // As `.messages { padding-bottom: calc(var(--dock-h) + 8px) }` on glass.
                ui.add_space(if floating { self.composer_height + 8.0 } else { 20.0 });
                if to_bottom {
                    ui.scroll_to_cursor(Some(Align::BOTTOM));
                }
            });
            // Whether the end of the conversation is in sight, to keep it there when the box
            // floating over it changes height.
            let end = (shown.content_size.y - shown.inner_rect.height()).max(0.0);
            self.at_end = to_bottom || shown.state.clamped_offset().y >= end - 4.0;
        });
        // The box gets room to reach its full height even when the list above was laid out
        // for a smaller one; the list makes way on the next frame, from the height measured.
        let room = if floating {
            let (bottom, column) = (list_top + list_height, list.response.rect);
            Rect::from_min_size(egui::pos2(column.left(), bottom - self.composer_height), Vec2::new(given.right() - column.left(), self.composer_height + COMPOSER_MAX))
        } else {
            Rect::from_min_size(ui.cursor().min, Vec2::new(given.right() - ui.cursor().min.x, composer_height.max(ui.available_height()) + COMPOSER_MAX))
        };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(room).layout(*ui.layout()));
        self.composer(&mut child, hub, palette, chrome);
        action
    }

    /// Files dropped on the window become the attachment of the next message.
    fn take_dropped(&mut self, ui: &Ui) {
        let dropped = ui.input(|input| input.raw.dropped_files.first().cloned());
        let Some(file) = dropped else { return };
        // With a thread open beside the conversation, the file goes where it was dropped.
        let here = match ui.ctx().pointer_latest_pos() {
            Some(pointer) => ui.max_rect().contains(pointer),
            None => self.thread.is_none(),
        };
        if !here {
            return;
        }
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
    fn message(
        &mut self,
        ui: &mut Ui,
        hub: &Arc<Hub>,
        pictures: &mut Pictures,
        styles: &Styles,
        palette: &Palette,
        chrome: &Chrome,
        message: &Message,
        first: bool,
        summary: Option<&ThreadSummary>,
        sticky: Option<&str>,
    ) -> Option<ChatAction> {
        let mut action = None;
        if first {
            ui.add_space(chrome.gap);
        }
        let member = if message.mine { Some(hub.member_id()) } else { message.member_id.clone() };
        let hover_fill = ui.painter().add(egui::Shape::Noop);
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
                        if message.is_pinned() {
                            let by = message.pinned_by().map_or_else(|| "Mensagem fixada".to_owned(), |by| format!("Fixada por {by}"));
                            ui.add(icons::PIN.image(13.0, palette.muted)).on_hover_text(by);
                        }
                    });
                }
                // A message that is only a GIF link shows the GIF without the link, as Discord.
                let link = first_link(&message.text);
                let source = link.as_deref().and_then(crate::media::animated_source);
                let moving = source.as_deref().and_then(|source| self.animated.get(ui.ctx(), source));
                // While the GIF comes the link stays hidden too, and a spinner holds its place.
                let waiting = moving.is_none() && source.as_deref().is_some_and(|source| self.animated.loading(source));
                let only_link = (moving.is_some() || waiting) && link.as_deref() == Some(message.text.trim());
                if !message.text.is_empty() && !only_link {
                    paint_text(ui, palette, &message.text, chrome.font);
                }
                if let Some(embed) = &message.embed {
                    if let Some(found) = self.embed(ui, hub, palette, message, embed) {
                        action = Some(found);
                    }
                } else if let Some((url, video)) = link.as_deref().and_then(|url| crate::media::video_of(url).map(|video| (url.to_owned(), video))) {
                    // A video link that came without a preview still plays here.
                    self.video_block(ui, palette, &format!("{}|{url}", message.id), &video, None);
                } else if let Some(found) = moving {
                    // The link came without a preview: the GIF is still fetched and plays.
                    let blob = Blob { uri: found.uri, bytes: found.bytes, size: found.size };
                    if let Some(found) = picture(ui, palette, Some(blob), IMAGE_BOUNDS, &format!("gif-{}.gif", message.sent_at)) {
                        action = Some(found);
                    }
                } else if waiting {
                    picture_waiting(ui, palette);
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
                if let Some(summary) = summary {
                    if thread_bar(ui, hub, pictures, palette, summary) {
                        action = Some(ChatAction::OpenThread(message.id.clone()));
                    }
                }
            });
        });

        // The tools of a message float at its top right while the cursor is over it, as in
        // Discord, and stay while the cursor is on them.
        let rect = Rect::from_min_max(egui::pos2(ui.max_rect().left(), row.response.rect.top()), egui::pos2(ui.max_rect().right(), row.response.rect.bottom()));
        let hovered = match sticky {
            Some(id) => id == message.id,
            None => ui.rect_contains_pointer(rect),
        };
        let now = ui.input(|input| input.time);
        let jumped = self.jump.as_mut().filter(|(id, _)| id == &message.id).map(|(_, reached)| {
            if reached.is_none() {
                ui.scroll_to_rect(rect, Some(Align::Center));
                *reached = Some(now);
            }
            reached.unwrap_or(now)
        });
        if let Some(reached) = jumped {
            // The message reached lights up and fades out.
            let light = (1.0 - (now - reached) as f32 / 1.8).clamp(0.0, 1.0);
            if light > 0.0 {
                ui.ctx().request_repaint();
                let lit = Rect::from_min_max(egui::pos2(rect.left() + 8.0, rect.top() - 4.0), egui::pos2(rect.right() - 8.0, rect.bottom() + 4.0));
                ui.painter().set(hover_fill, egui::Shape::rect_filled(lit, CornerRadius::same(if palette.glass { 12 } else { 4 }), palette.accent.gamma_multiply(0.22 * light)));
            } else {
                self.jump = None;
            }
        } else if hovered {
            // `.message:hover`: a faint light under the message.
            let lit = if palette.glass {
                Rect::from_min_max(egui::pos2(rect.left() + 8.0, rect.top() - 8.0), egui::pos2(rect.right() - 8.0, rect.bottom() + 8.0))
            } else {
                Rect::from_min_max(egui::pos2(rect.left(), rect.top() - 1.0), egui::pos2(rect.right(), rect.bottom() + 1.0))
            };
            let fill = if palette.glass { Color32::from_white_alpha(10) } else { palette.hover.gamma_multiply(0.5) };
            ui.painter().set(hover_fill, egui::Shape::rect_filled(lit, CornerRadius::same(if palette.glass { 12 } else { 0 }), fill));
        }
        let in_thread_view = self.thread.is_some();
        let mut tools: Vec<(Tool, icons::Icon, &str)> = Vec::new();
        if !in_thread_view {
            tools.push((Tool::Thread, icons::THREAD, "Responder em tópico"));
        }
        tools.push(if message.is_pinned() { (Tool::Unpin, icons::PIN, "Desafixar mensagem") } else { (Tool::Pin, icons::PIN, "Fixar mensagem") });
        if message.mine {
            tools.push((Tool::Delete, icons::DELETE, "Apagar mensagem (Shift apaga sem perguntar)"));
        }
        let mut chosen = None;
        if hovered {
            let size = 30.0;
            let width = tools.len() as f32 * size + 8.0;
            let top = (rect.top() - 16.0).max(ui.clip_rect().top() + 2.0);
            let bar = Rect::from_min_size(egui::pos2(rect.right() - 16.0 - width, top), Vec2::new(width, size + 8.0));
            self.tools_for = Some((message.id.clone(), bar));
            let round = CornerRadius::same(if palette.glass { 12 } else { 6 });
            if palette.glass {
                let pane = crate::glass::Pane { blur: Some(12.0), shine: false, ..crate::glass::Pane::new(Color32::from_rgba_unmultiplied(40, 42, 64, 200), 12.0) };
                crate::glass::pane(ui.painter(), bar, &pane);
            } else {
                ui.painter().add(egui::Shadow { offset: [0, 2], blur: 8, spread: 0, color: Color32::from_black_alpha(60) }.as_shape(bar, round));
                ui.painter().rect_filled(bar, round, palette.panel);
                ui.painter().rect_stroke(bar, round, Stroke::new(1.0, palette.line), egui::StrokeKind::Inside);
            }
            for (index, (tool, icon, tip)) in tools.iter().enumerate() {
                let button = Rect::from_min_size(bar.min + Vec2::new(4.0 + index as f32 * size, 4.0), Vec2::splat(size));
                let response = ui.interact(button, ui.scope_id().with(("tool", &message.id, index)), Sense::click());
                let lit = crate::motion::hover(ui.ctx(), response.id, response.hovered());
                let danger = *tool == Tool::Delete;
                let over = if danger { palette.danger } else { palette.selected };
                if lit > 0.0 {
                    ui.painter().rect_filled(button.shrink(1.0), CornerRadius::same(if palette.glass { 8 } else { 4 }), over.gamma_multiply(lit));
                }
                let rest = if *tool == Tool::Unpin { palette.accent_text } else { palette.soft };
                let ink = crate::motion::mix(rest, if danger { Color32::WHITE } else { palette.strong }, lit);
                icon.image(18.0, ink).paint_at(ui, Rect::from_center_size(button.center(), Vec2::splat(18.0)));
                if response.on_hover_text(*tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    chosen = Some(*tool);
                }
            }
        }
        let menu = ui.interact(rect, ui.scope_id().with(("menu", &message.id)), Sense::hover());
        // The text cannot be selected here as it could there, so the menu also copies it.
        crate::app::context_menu(&menu, palette, |ui| {
            if !in_thread_view && crate::app::menu_item(ui, palette, "Responder em tópico", false) {
                chosen = Some(Tool::Thread);
            }
            let pin = if message.is_pinned() { "Desafixar mensagem" } else { "Fixar mensagem" };
            if crate::app::menu_item(ui, palette, pin, false) {
                chosen = Some(if message.is_pinned() { Tool::Unpin } else { Tool::Pin });
            }
            if !message.text.is_empty() && crate::app::menu_item(ui, palette, "Copiar texto", false) {
                ui.ctx().copy_text(message.text.clone());
            }
            if message.mine {
                crate::app::menu_separator(ui, palette);
                if crate::app::menu_item(ui, palette, "Apagar mensagem", true) {
                    chosen = Some(Tool::Delete);
                }
            }
        });
        match chosen {
            Some(Tool::Thread) => action = Some(ChatAction::OpenThread(message.id.clone())),
            Some(Tool::Pin) => self.set_pinned(hub, &message.id, true),
            Some(Tool::Unpin) => self.set_pinned(hub, &message.id, false),
            Some(Tool::Delete) => {
                let ask = !ui.input(|input| input.modifiers.shift);
                action = Some(ChatAction::Delete { message_id: message.id.clone(), ask });
            }
            None => {}
        }
        action
    }

    fn embed(&mut self, ui: &mut Ui, hub: &Arc<Hub>, palette: &Palette, message: &Message, embed: &Embed) -> Option<ChatAction> {
        let mut action = None;
        let video = crate::media::video_of(&embed.url);
        // A GIF link plays: the moving picture is fetched here, in place of the still that came.
        let source = crate::media::animated_source(&embed.url);
        let moving = source.as_deref().and_then(|source| self.animated.get(ui.ctx(), source));
        let waiting = moving.is_none() && source.as_deref().is_some_and(|source| self.animated.loading(source));
        let moving = moving.map(|found| Blob { uri: found.uri, bytes: found.bytes, size: found.size });
        let only_picture = embed.title.is_empty() && embed.description.is_empty() || crate::media::gif_site(&embed.url);
        if waiting && only_picture {
            picture_waiting(ui, palette);
            return None;
        }
        if let Some(blob) = &moving {
            // A link that is only the picture shows only the picture, as Discord does.
            if only_picture {
                return picture(ui, palette, Some(blob.clone()), IMAGE_BOUNDS, &format!("gif-{}.gif", message.sent_at));
            }
        }
        ui.add_space(4.0);
        let glass = palette.glass.then(|| crate::glass::reserve(ui.painter()));
        let frame = if palette.glass {
            Frame::new().corner_radius(CornerRadius::same(12)).inner_margin(Margin { left: 17, right: 15, top: 11, bottom: 13 })
        } else {
            Frame::new().fill(palette.panel).corner_radius(CornerRadius::same(6)).inner_margin(Margin { left: 16, right: 14, top: 10, bottom: 12 })
        };
        let inner = frame.show(ui, |ui| {
            ui.set_max_width(432.0f32.min(ui.available_width()));
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
            if let Some(video) = &video {
                let still = embed.image.as_ref().and_then(|image| self.image_blob(hub, image));
                self.video_block(ui, palette, &format!("{}|{}", message.id, embed.url), video, still);
            } else if let Some(blob) = moving {
                if let Some(found) = picture(ui, palette, Some(blob), EMBED_BOUNDS, &format!("gif-{}.gif", message.sent_at)) {
                    action = Some(found);
                }
            } else if let Some(image) = &embed.image {
                let blob = self.image_blob(hub, image);
                if let Some(found) = picture(ui, palette, blob, EMBED_BOUNDS, &image_file_name(&image.mime, message.sent_at)) {
                    action = Some(found);
                }
            }
        });
        let rect = inner.response.rect;
        if let Some(reserved) = glass {
            // `.embed`: white at 6%, a light border, 3 points of accent on the left that follow
            // the round corners, and a blur of 12.
            let pane = crate::glass::Pane { blur: Some(12.0), shine: false, edge: false, ..crate::glass::Pane::new(Color32::from_white_alpha(15), 12.0) };
            crate::glass::fill(ui.painter(), reserved, rect, &pane);
            ui.painter().rect_stroke(rect, CornerRadius::same(12), Stroke::new(1.0, palette.line_soft), egui::StrokeKind::Inside);
            let strip = Rect::from_min_max(rect.left_top(), egui::pos2(rect.left() + 3.0, rect.bottom()));
            ui.painter().with_clip_rect(strip.intersect(ui.clip_rect())).rect_filled(rect, CornerRadius::same(12), palette.accent);
        } else {
            let bar = Rect::from_min_max(rect.left_top(), rect.left_bottom() + Vec2::new(4.0, 0.0));
            ui.painter().rect_filled(bar, CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 }, palette.accent);
        }
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
        let mut nudge = false;
        let drawn = Frame::new().inner_margin(Margin { left: 16, right: 16, top: 0, bottom: 0 }).show(ui, |ui| {
            self.pending_strip(ui, palette);
            self.link.show(ui, palette);
            let glass = palette.glass.then(|| crate::glass::reserve(ui.painter()));
            let frame = if palette.glass {
                Frame::new().corner_radius(CornerRadius::same(14)).inner_margin(Margin::symmetric(8, 4))
            } else {
                Frame::new().fill(palette.raised).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(12, 7))
            };
            let boxed = frame.show(ui, |ui| {
                // As `.composer textarea { padding: 9px 4px }` and its line: a row 41 points
                // high on glass, the box 49 with its border.
                let row = if palette.glass { 41.0 } else { 0.0 };
                ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), row), Layout::left_to_right(Align::Center), |ui| {
                    if palette.glass {
                        ui.set_min_height(row);
                    }
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let attach = widgets::icon_button(ui, icons::ATTACH, if palette.glass { 32.0 } else { 28.0 }, palette.soft, palette.selected, "Anexar um arquivo");
                    let plus = Rect::from_center_size(attach.rect.center(), Vec2::splat(12.0));
                    // The plus is cut out of the circle: the color of the box under it.
                    let cut = if palette.glass { Color32::from_rgb(40, 42, 64) } else { palette.raised };
                    ui.painter().line_segment([plus.center_top(), plus.center_bottom()], Stroke::new(2.0, cut));
                    ui.painter().line_segment([plus.left_center(), plus.right_center()], Stroke::new(2.0, cut));
                    if attach.clicked() {
                        if let Some(path) = attach::pick_file() {
                            self.attach(attach::from_path(&path));
                        }
                    }
                    // Only a conversation between two people can call for attention, as in MSN.
                    let direct = self.thread.is_none() && self.conversation.as_ref().is_some_and(|conversation| conversation.channel_id.is_none());
                    let width = ui.available_width() - if direct { 82.0 } else { 40.0 };
                    let edit = egui::TextEdit::multiline(&mut self.draft)
                        .hint_text(RichText::new(&chrome.placeholder).color(palette.muted))
                        .font(FontId::proportional(CHAT_FONT))
                        .text_color(palette.text)
                        .frame(Frame::new())
                        .desired_rows(1)
                        .desired_width(width)
                        .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter));
                    // Grows with the draft up to 200 px, as `.composer-input`, then scrolls,
                    // with the bar in sight and clear of the text.
                    let scroll = &mut ui.spacing_mut().scroll;
                    scroll.dormant_handle_opacity = 0.6;
                    scroll.dormant_background_opacity = 0.25;
                    let edit = edit.desired_width(width - 12.0);
                    let scrolled = egui::ScrollArea::vertical()
                        .id_salt("composer-input")
                        .max_height(COMPOSER_MAX)
                        .min_scrolled_height(self.draft_height.min(COMPOSER_MAX))
                        .auto_shrink([true, true])
                        .show(ui, |ui| ui.add(edit));
                    // The height of the text itself: what the scroll area holds never shrinks
                    // below the height it was given, so the box would stay tall after a delete.
                    let needed = scrolled.inner.rect.height();
                    if (needed - self.draft_height).abs() > 0.5 {
                        self.draft_height = needed;
                        ui.ctx().request_repaint();
                    }
                    let response = scrolled.inner;
                    if std::mem::take(&mut self.focus_input) {
                        response.request_focus();
                    }
                    self.focused = response.has_focus();
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
                        if widgets::icon_button(ui, icons::SEND, if palette.glass { 32.0 } else { 28.0 }, ink, palette.selected, "Enviar").clicked() {
                            send = true;
                        }
                        if direct && widgets::icon_button(ui, icons::BELL, if palette.glass { 32.0 } else { 28.0 }, palette.soft, palette.selected, "Chamar a atenção").clicked() {
                            nudge = true;
                        }
                    });
                });
            });
            self.box_rect = Some(boxed.response.rect);
            if let Some(reserved) = glass {
                let rect = boxed.response.rect;
                let shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(64) };
                let pane = crate::glass::Pane { shadow: Some(shadow), ..crate::glass::Pane::new(Color32::from_white_alpha(15), 14.0) };
                crate::glass::fill(ui.painter(), reserved, rect, &pane);
                if self.focused {
                    // As `.composer:focus-within`: an accent border and a soft ring around it.
                    ui.painter().rect_stroke(rect, CornerRadius::same(14), Stroke::new(1.0, Color32::from_rgba_unmultiplied(109, 114, 255, 179)), egui::StrokeKind::Inside);
                    ui.painter().rect_stroke(rect, CornerRadius::same(14), Stroke::new(3.0, Color32::from_rgba_unmultiplied(109, 114, 255, 51)), egui::StrokeKind::Outside);
                }
            }
            ui.add_space(if palette.glass { 0.0 } else { 4.0 });
            let long = too_long(self.draft.trim()).then(|| {
                let count = self.draft.trim().encode_utf16().count();
                format!("{count}/{MAX_TEXT_LENGTH} caracteres: o texto vai como o arquivo mensagem.txt.")
            });
            let hint = if !self.hint.is_empty() { &self.hint } else { long.as_ref().unwrap_or(&chrome.hint) };
            if palette.glass {
                // The note under the box is glass too, across the whole conversation, so the
                // text passing behind does not get in the way: `.footnote { padding: 6px 16px 8px }`.
                // The band starts right under the box, with no gap for the text behind to show.
                let reserved = crate::glass::reserve(ui.painter());
                let top = boxed.response.rect.bottom();
                let gap = ui.spacing().item_spacing.y;
                ui.add_space(6.0 - gap);
                let note = ui.label(RichText::new(hint).color(palette.muted).size(12.0));
                // A line of 17 points under a 12 point font, as the browser draws it.
                ui.add_space(note.rect.top() + note.rect.height().max(17.0) + 8.0 - ui.cursor().min.y);
                let whole = ui.ctx().content_rect();
                let left = ui.max_rect().left() - 16.0;
                let right = ui.max_rect().right() + 16.0;
                let round = |touches: bool| if touches { 16 } else { 0 };
                // A thread beside the conversation has the conversation at its left, not a corner.
                let corners = CornerRadius { nw: 0, ne: 0, sw: if self.thread.is_some() { 0 } else { 16 }, se: round((whole.right() - right) < 12.0) };
                let band = Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, ui.cursor().min.y));
                let pane = crate::glass::Pane { corners, edge: false, shine: false, ..crate::glass::Pane::new(Color32::TRANSPARENT, 0.0) };
                crate::glass::fill(ui.painter(), reserved, band, &pane);
            } else {
                ui.label(RichText::new(hint).color(palette.muted).size(12.0));
            }
        });
        let height = drawn.response.rect.height();
        if (height - self.composer_height).abs() > 0.5 {
            self.composer_height = height;
            if palette.glass && self.at_end && self.scroll_test.is_none() {
                self.to_bottom = true;
            }
            ui.ctx().request_repaint();
        }
        if nudge {
            let context = ui.ctx().clone();
            self.send_nudge(hub, &context);
        }
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

/// More than one message carries; the store would cut it at the limit.
fn too_long(text: &str) -> bool {
    text.encode_utf16().nth(MAX_TEXT_LENGTH).is_some()
}

fn same_block(previous: &Message, message: &Message) -> bool {
    previous.mine == message.mine
        && previous.author == message.author
        && previous.member_id == message.member_id
        && message.sent_at - previous.sent_at < GROUP_WINDOW_MS
}

/// The place of a GIF that is still on the way.
fn picture_waiting(ui: &mut Ui, palette: &Palette) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(220.0, 160.0), Sense::hover());
    let round = CornerRadius::same(if palette.glass { 14 } else { 8 });
    ui.painter().rect_filled(rect, round, palette.rail);
    egui::Spinner::new().size(22.0).color(palette.muted).paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(22.0)));
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
    // A narrow column, as a thread's, makes the picture smaller instead of cutting it.
    let bounds = Vec2::new(bounds.x.min(ui.available_width().max(120.0)), bounds.y);
    let scale = (bounds.x / blob.size.x).min(bounds.y / blob.size.y).min(1.0);
    let shown = (blob.size * scale).max(Vec2::splat(60.0));
    let (rect, response) = ui.allocate_exact_size(shown, Sense::click());
    // On glass the pictures are rounder and float a little: `.image { border-radius: 14px }`.
    let round = CornerRadius::same(if palette.glass { 14 } else { 8 });
    if palette.glass {
        ui.painter().add(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(77) }.as_shape(rect, round));
    }
    ui.painter().rect_filled(rect, round, palette.rail);
    egui::Image::from_bytes(blob.uri.clone(), egui::load::Bytes::Shared(Arc::clone(&blob.bytes))).corner_radius(round).paint_at(ui, rect);

    let mut action = None;
    crate::app::context_menu(&response, palette, |ui| {
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

/// What the tools over a message do.
#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Thread,
    Pin,
    Unpin,
    Delete,
}

/// The bar under a message with answers, as Slack: the faces of who answered, how many answers
/// and when the last came. True when it was clicked, to open the thread.
fn thread_bar(ui: &mut Ui, hub: &Hub, pictures: &mut Pictures, palette: &Palette, summary: &ThreadSummary) -> bool {
    ui.add_space(4.0);
    let count = if summary.count == 1 { "1 resposta".to_owned() } else { format!("{} respostas", summary.count) };
    let count_galley = ui.painter().layout_no_wrap(count, FontId::new(13.0, theme::semibold()), palette.link);
    let last = format!("Última resposta {}", format_time(summary.last_at).to_lowercase());
    let last_galley = ui.painter().layout_no_wrap(last, FontId::proportional(12.0), palette.muted);
    let hint_galley = ui.painter().layout_no_wrap("Ver tópico".to_owned(), FontId::proportional(12.0), palette.muted);
    let faces = summary.faces.len() as f32;
    let width = 6.0 + 22.0 + (faces - 1.0).max(0.0) * 18.0 + 14.0 + count_galley.size().x + 10.0 + last_galley.size().x.max(hint_galley.size().x) + 12.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width.min(ui.available_width()), 32.0), Sense::click());
    let lit = crate::motion::hover(ui.ctx(), response.id, response.hovered());
    let round = CornerRadius::same(if palette.glass { 10 } else { 6 });
    if lit > 0.0 {
        let fill = if palette.glass { Color32::from_white_alpha((15.0 * lit) as u8) } else { palette.panel.gamma_multiply(lit) };
        ui.painter().rect_filled(rect, round, fill);
        let edge = if palette.glass { crate::glass::EDGE } else { palette.line };
        ui.painter().rect_stroke(rect, round, Stroke::new(1.0, edge.gamma_multiply(lit)), egui::StrokeKind::Inside);
    }
    // The faces overlap a little, each with a ring of the background around it.
    let ring = if palette.glass { Color32::from_rgb(30, 31, 48) } else { palette.main };
    let mut x = rect.left() + 6.0;
    for (member, name) in &summary.faces {
        let face = Rect::from_center_size(egui::pos2(x + 11.0, rect.center().y), Vec2::splat(22.0));
        ui.painter().circle_filled(face.center(), 12.5, ring);
        widgets::paint_avatar(ui, face, name, pictures.avatar(hub, member.as_deref()));
        x += 18.0;
    }
    x += 4.0 + 10.0;
    ui.painter().galley(egui::pos2(x, rect.center().y - count_galley.size().y / 2.0), count_galley.clone(), palette.link);
    x += count_galley.size().x + 10.0;
    // On hover the time gives way to the invitation, as Slack does.
    if lit < 1.0 {
        ui.painter().galley_with_override_text_color(egui::pos2(x, rect.center().y - last_galley.size().y / 2.0), last_galley, palette.muted.gamma_multiply(1.0 - lit));
    }
    if lit > 0.0 {
        ui.painter().galley_with_override_text_color(egui::pos2(x, rect.center().y - hint_galley.size().y / 2.0), hint_galley, palette.strong.gamma_multiply(lit));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The line under the message a thread hangs from: how many answers, and a rule.
fn thread_divider(ui: &mut Ui, palette: &Palette, count: usize) {
    ui.add_space(10.0);
    let label = match count {
        0 => "Nenhuma resposta ainda".to_owned(),
        1 => "1 resposta".to_owned(),
        count => format!("{count} respostas"),
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 18.0), Sense::hover());
    let galley = ui.painter().layout_no_wrap(label, FontId::new(12.0, theme::semibold()), palette.muted);
    let left = rect.left() + 16.0;
    let width = galley.size().x;
    ui.painter().galley(egui::pos2(left, rect.center().y - galley.size().y / 2.0), galley, palette.muted);
    let line_left = left + width + 10.0;
    if line_left < rect.right() - 16.0 {
        ui.painter().hline(line_left..=rect.right() - 16.0, rect.center().y, Stroke::new(1.0, palette.line_soft));
    }
    ui.add_space(2.0);
}
