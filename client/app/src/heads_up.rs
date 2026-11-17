//! The cards that arrive at the top right of the window while it is in front: a message from a
//! conversation that is not open, a mention, a call ringing, a share that began in the room.
//! As `.notices` in `design/sketch`. With the window behind others Windows shows its own notice.

use std::collections::HashMap;

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, CornerRadius, FontId, Id, Order, Pos2, Rect, Sense, Shape, Stroke, Ui, UiBuilder, Vec2};
use telinha_core::chats::Message;
use telinha_core::hub::{Conversation, DM_ROOM, Outgoing};

use crate::app::{Nav, TelinhaApp};
use crate::icons::{self, Icon};
use crate::room_view::Person;
use crate::theme::{self, Palette};
use crate::widgets;

const WIDTH: f32 = 340.0;
const RADIUS: u8 = 12;
const PAD: f32 = 14.0;
/// Where the text, the reply and the buttons start: past the avatar and its gap.
const INDENT: f32 = 52.0;
/// How long a card stays; the pointer over it, or a reply being written, holds the time.
const LASTS: f64 = 8.0;
/// The slide in from the right edge.
const SLIDE: f64 = 0.34;
const MOST: usize = 3;
/// The card of the call ringing has this id; the others count up from zero.
const CALL: u64 = u64::MAX;

#[derive(Clone, PartialEq)]
enum What {
    Message { conversation: Conversation, mention: bool },
    Live { space_id: String, room_id: String, peer_id: String },
}

struct Card {
    id: u64,
    what: What,
    name: String,
    member_id: Option<String>,
    icon: Icon,
    place: String,
    text: String,
    arrived: f64,
    left: f64,
    reply: String,
}

#[derive(Default)]
pub struct HeadsUp {
    cards: Vec<Card>,
    next: u64,
    /// The height of each card in the last frame, to stack the next ones under it.
    heights: HashMap<u64, f32>,
    last_frame: Option<f64>,
    /// The time stands still, for the pictures of the tests, whose clock jumps.
    hold: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Danger,
    Green,
    Accent,
}

enum Tail<'a> {
    Reply { draft: &'a mut String, to: &'a str },
    Buttons(Vec<(Icon, &'static str, Tone)>),
}

struct Face<'a> {
    id: u64,
    icon: Icon,
    place: &'a str,
    arrived: f64,
    name: &'a str,
    picture: Option<egui::Image<'static>>,
    style: Option<&'a telinha_core::profile::Style>,
    live: bool,
    ringing: bool,
    mention: Option<String>,
    text: &'a str,
    tail: Tail<'a>,
    /// What is left of the time, from 1 to 0, for the cards that go away by themselves.
    time: Option<f32>,
}

#[derive(Clone, Copy, PartialEq)]
enum Click {
    Close,
    Open,
    Button(usize),
    Send,
}

struct Drawn {
    height: f32,
    held: bool,
    click: Option<Click>,
}

impl TelinhaApp {
    /// A message arrived: it gets a card when the window is in front and its conversation is
    /// not the one open. A new message of a conversation that has a card takes its place.
    pub(crate) fn heads_up_message(&mut self, conversation: &Conversation, message: &Message) {
        if message.mine || message.is_nudge() || !self.window_in_front() || self.conversation().as_ref() == Some(conversation) {
            return;
        }
        let (place, icon) = match self.servers.iter().find(|server| server.id == conversation.space_id) {
            Some(server) => {
                let channel = conversation.channel_id.as_deref().and_then(|id| server.channels.iter().find(|channel| channel.id == id));
                (channel.map_or_else(|| server.name.clone(), |channel| format!("{} · {}", channel.name, server.name)), icons::HASH)
            }
            None => ("Mensagem direta".to_owned(), icons::CHAT),
        };
        let text = if !message.text.trim().is_empty() {
            message.text.split_whitespace().collect::<Vec<_>>().join(" ")
        } else if let Some(file) = &message.file {
            format!("Mandou {}", file.name)
        } else {
            "Mandou uma imagem".to_owned()
        };
        let me = self.hub.profile_name();
        let mention = !me.is_empty() && text.to_lowercase().contains(&format!("@{}", me.to_lowercase()));
        let what = What::Message { conversation: conversation.clone(), mention };
        self.heads_up.cards.retain(|card| !matches!(&card.what, What::Message { conversation: known, .. } if known == conversation));
        self.push_card(what, message.author.clone(), message.member_id.clone(), icon, place, text);
    }

    /// Someone in the room started sharing their screen while the person looks elsewhere.
    pub(crate) fn heads_up_live(&mut self, space_id: &str, room_id: &str, sharer: &Person) {
        if !self.window_in_front() {
            return;
        }
        let looking = match &self.nav {
            Nav::Direct(contact_id) => room_id == DM_ROOM && contact_id == space_id,
            Nav::Server { server_id, channel_id } => server_id == space_id && channel_id.as_deref() == Some(room_id),
            Nav::Home => false,
        };
        if looking {
            return;
        }
        let place = match self.servers.iter().find(|server| server.id == space_id) {
            Some(server) => {
                let room = server.channels.iter().find(|channel| channel.id == room_id).map_or("Sala", |channel| channel.name.as_str());
                format!("{room} · {}", server.name)
            }
            None => format!("Chamada com {}", sharer.name),
        };
        let what = What::Live { space_id: space_id.to_owned(), room_id: room_id.to_owned(), peer_id: sharer.id.clone() };
        self.heads_up.cards.retain(|card| card.what != what);
        self.push_card(what, sharer.name.clone(), sharer.member_id.clone(), icons::SCREEN, place, "começou a transmitir a tela".to_owned());
    }

    fn push_card(&mut self, what: What, name: String, member_id: Option<String>, icon: Icon, place: String, text: String) {
        let arrived = self.context.input(|input| input.time);
        let id = self.heads_up.next;
        self.heads_up.next += 1;
        self.heads_up.cards.insert(0, Card { id, what, name, member_id, icon, place, text, arrived, left: LASTS, reply: String::new() });
        self.heads_up.cards.truncate(MOST);
        self.context.request_repaint();
    }

    /// Puts cards up as they look after they arrived, for the pictures of the tests: a direct
    /// message, a mention in the last text channel, and a share in the last voice channel.
    pub fn heads_up_for_test(&mut self, kinds: &[&str]) {
        self.heads_up.cards.clear();
        self.heads_up.hold = !kinds.is_empty();
        let now = self.context.input(|input| input.time);
        for kind in kinds.iter().rev() {
            let server = self.servers.first().cloned();
            let channel = |voice: bool| {
                server.as_ref().and_then(|server| server.channels.iter().rev().find(|channel| (channel.kind == telinha_core::servers::ChannelKind::Voice) == voice).cloned())
            };
            let (what, name, icon, place, text) = match *kind {
                "mention" => {
                    let (server, channel) = (server.clone().unwrap(), channel(false).unwrap());
                    let text = format!("@{} entra na sala, só falta você pra começar", self.hub.profile_name());
                    (What::Message { conversation: Conversation::channel(&server.id, &channel.id), mention: true }, "Keep", icons::HASH, format!("{} · {}", channel.name, server.name), text)
                }
                "live" => {
                    let (server, room) = (server.clone().unwrap(), channel(true).unwrap());
                    let what = What::Live { space_id: server.id.clone(), room_id: room.id.clone(), peer_id: String::new() };
                    (what, "Keep", icons::SCREEN, format!("{} · {}", room.name, server.name), "começou a transmitir a tela".to_owned())
                }
                _ => {
                    let text = "chegou o bolo, vem buscar antes que o Keep coma tudo".to_owned();
                    (What::Message { conversation: Conversation::direct("test"), mention: false }, "nanda", icons::CHAT, "Mensagem direta".to_owned(), text)
                }
            };
            let member_id = self.hub.peers(&server.as_ref().map_or(String::new(), |server| server.id.clone())).into_iter().find(|peer| peer.name == name).and_then(|peer| peer.member_id);
            self.push_card(what, name.to_owned(), member_id, icon, place, text);
            if let Some(card) = self.heads_up.cards.first_mut() {
                card.arrived = now - 2.0;
                card.left = LASTS * 0.6;
            }
        }
    }

    /// Draws the cards, the call ringing on top, and does what was clicked on them.
    pub(crate) fn show_heads_up(&mut self, context: &egui::Context) {
        let now = context.input(|input| input.time);
        let elapsed = self.heads_up.last_frame.map_or(0.0, |last| (now - last).clamp(0.0, 0.25));
        self.heads_up.last_frame = Some(now);
        // A card of the conversation that was opened has done its work.
        let open = self.conversation();
        self.heads_up.cards.retain(|card| !matches!(&card.what, What::Message { conversation, .. } if Some(conversation) == open.as_ref()));

        let palette = self.palette;
        let gap = if crate::glass::look(context).is_some() { 10.0 } else { 0.0 };
        let screen = context.content_rect();
        let right = screen.right() - gap - 16.0;
        let mut top = screen.top() + gap + self.prefs.appearance.density.header_height() + 12.0;
        let mut ids = Vec::new();

        if let Some((contact_id, name, live, started)) = self.ringing() {
            let member_id = self.hub.peers(&contact_id).into_iter().find_map(|peer| peer.member_id);
            let picture = self.pictures.avatar(&self.hub, member_id.as_deref());
            let (icon, place, text) =
                if live { (icons::SCREEN, "Transmissão", "está transmitindo a tela para você") } else { (icons::PHONE, "Chamada de voz", "está te ligando") };
            let accept = if live { (icons::SCREEN, "Assistir", Tone::Accent) } else { (icons::PHONE, "Atender", Tone::Green) };
            let face = Face {
                id: CALL,
                icon,
                place,
                arrived: started,
                name: &name,
                picture,
                style: self.styles.of(member_id.as_deref()),
                live,
                ringing: true,
                mention: None,
                text,
                tail: Tail::Buttons(vec![(icons::HANGUP, "Recusar", Tone::Danger), accept]),
                time: None,
            };
            let drawn = card(context, &palette, Pos2::new(right - WIDTH, top), now, self.heads_up.heights.get(&CALL).copied(), face);
            self.heads_up.heights.insert(CALL, drawn.height);
            top += drawn.height + 10.0;
            ids.push(CALL);
            match drawn.click {
                Some(Click::Close | Click::Button(0)) => self.decline_call(),
                Some(Click::Open | Click::Button(_)) => self.answer_call(),
                _ => {}
            }
        }

        let mut cards = std::mem::take(&mut self.heads_up.cards);
        let room = if self.ringing().is_some() { MOST - 1 } else { MOST };
        cards.truncate(room);
        let me = self.hub.profile_name();
        let mut done = Vec::new();
        for card_data in &mut cards {
            let picture = self.pictures.avatar(&self.hub, card_data.member_id.as_deref());
            let (tail, mention) = match &card_data.what {
                What::Message { mention, .. } => (Tail::Reply { draft: &mut card_data.reply, to: &card_data.name }, mention.then(|| format!("@{me}"))),
                What::Live { .. } => (Tail::Buttons(vec![(icons::SCREEN, "Assistir", Tone::Accent)]), None),
            };
            let face = Face {
                id: card_data.id,
                icon: card_data.icon,
                place: &card_data.place,
                arrived: card_data.arrived,
                name: &card_data.name,
                picture,
                style: self.styles.of(card_data.member_id.as_deref()),
                live: matches!(card_data.what, What::Live { .. }),
                ringing: false,
                mention,
                text: &card_data.text,
                tail,
                time: Some((card_data.left / LASTS).clamp(0.0, 1.0) as f32),
            };
            let drawn = card(context, &palette, Pos2::new(right - WIDTH, top), now, self.heads_up.heights.get(&card_data.id).copied(), face);
            self.heads_up.heights.insert(card_data.id, drawn.height);
            top += drawn.height + 10.0;
            ids.push(card_data.id);
            if !drawn.held && !self.heads_up.hold {
                card_data.left -= elapsed;
            }
            if card_data.left <= 0.0 {
                done.push((card_data.id, None));
            } else if let Some(click) = drawn.click {
                done.push((card_data.id, Some(click)));
            }
        }
        if !cards.is_empty() || self.ringing().is_some() {
            context.request_repaint();
        }
        self.heads_up.heights.retain(|id, _| ids.contains(id));

        for (id, click) in done {
            let Some(index) = cards.iter().position(|card| card.id == id) else { continue };
            // Enter with nothing written leaves the card as it was.
            if click == Some(Click::Send) && cards[index].reply.trim().is_empty() && cards[index].left > 0.0 {
                continue;
            }
            let card = cards.remove(index);
            match (card.what, click) {
                (_, None | Some(Click::Close)) => {}
                (What::Message { conversation, .. }, Some(Click::Send)) => {
                    let text = card.reply.trim().to_owned();
                    match self.hub.send_chat(&conversation, Outgoing { text, ..Outgoing::default() }) {
                        Ok(Some(message)) => {
                            self.chat.received(&conversation, message);
                            self.notice(format!("Resposta enviada para {}.", card.name));
                        }
                        Ok(None) => {}
                        Err(error) => self.notice(format!("Não consegui enviar: {error}.")),
                    }
                }
                (What::Message { conversation, .. }, Some(_)) => self.open_conversation(&conversation),
                (What::Live { space_id, room_id, peer_id }, Some(_)) => {
                    let nav = if room_id == DM_ROOM { Nav::Direct(space_id) } else { Nav::Server { server_id: space_id, channel_id: Some(room_id) } };
                    self.navigate(nav);
                    if !peer_id.is_empty()
                        && let Err(error) = self.hub.watch(&peer_id)
                    {
                        self.notice(error);
                    }
                }
            }
        }
        // Cards that arrived while these were drawn go on top of them.
        cards.splice(0..0, std::mem::take(&mut self.heads_up.cards));
        cards.truncate(MOST);
        self.heads_up.cards = cards;
    }
}

/// How far the card still is from its place, as `cubic-bezier(.2, .9, .3, 1.15)`: fast, with a
/// small overshoot at the end.
fn slide(seconds: f64) -> f32 {
    let t = (seconds / SLIDE).clamp(0.0, 1.0) as f32;
    let (c1, c3) = (0.9, 1.9);
    let eased = 1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2);
    (1.0 - eased) * (WIDTH + 32.0)
}

fn card(context: &egui::Context, palette: &Palette, at: Pos2, now: f64, height: Option<f32>, face: Face<'_>) -> Drawn {
    let origin = at + Vec2::new(slide(now - face.arrived), 0.0);
    let id = Id::unique(("heads-up", face.id));
    let mut drawn = Drawn { height: height.unwrap_or(120.0), held: false, click: None };
    egui::Area::new(id).order(Order::Foreground).fixed_pos(origin).constrain(false).show(context, |ui| {
        ui.set_width(WIDTH);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let painter = ui.painter().clone();
        let look = crate::glass::look(context);
        let reserved = look.map(|_| crate::glass::reserve(&painter));
        let (shadow_slot, back_slot) = (painter.add(Shape::Noop), painter.add(Shape::Noop));
        // The card itself takes the click; the buttons and the field on it come after and win.
        let back = ui.interact(Rect::from_min_size(origin, Vec2::new(WIDTH, drawn.height)), id.with("back"), Sense::click());
        let back = back.on_hover_cursor(egui::CursorIcon::PointingHand);
        if back.clicked() {
            drawn.click = Some(Click::Open);
        }

        // The line of where it came from, the time and the close button.
        let line = Rect::from_min_size(origin + Vec2::new(PAD, 12.0), Vec2::new(WIDTH - 2.0 * PAD, 20.0));
        face.icon.image(14.0, palette.muted).paint_at(ui, Rect::from_center_size(line.left_center() + Vec2::new(7.0, 0.0), Vec2::splat(14.0)));
        let close_rect = Rect::from_center_size(egui::pos2(line.right() - 5.0, line.center().y), Vec2::splat(22.0));
        let close = ui.scope_builder(UiBuilder::new().max_rect(close_rect), |ui| widgets::icon_button(ui, icons::CLOSE, 22.0, palette.muted, palette.hover, "Fechar")).inner;
        if close.clicked() {
            drawn.click = Some(Click::Close);
        }
        let ago = now - face.arrived;
        let when = if ago < 60.0 { "agora".to_owned() } else { format!("há {} min", (ago / 60.0).floor()) };
        let when = painter.layout_no_wrap(when, FontId::proportional(12.0), palette.muted);
        let when_left = close_rect.left() - 8.0 - when.size().x;
        painter.galley(egui::pos2(when_left, line.center().y - when.size().y / 2.0), when, palette.muted);
        let place_left = line.left() + 20.0;
        widgets::paint_text_clipped(ui, egui::pos2(place_left, line.center().y), (when_left - 8.0 - place_left).max(0.0), face.place, FontId::new(12.0, theme::semibold()), palette.muted);

        // The face, the name and the text.
        let top = line.bottom() + 10.0;
        let avatar = Rect::from_min_size(egui::pos2(origin.x + PAD, top), Vec2::splat(40.0));
        widgets::paint_avatar(ui, avatar, face.name, face.picture);
        if face.ringing {
            // A green ring, and a second one that grows and fades, until the call is answered.
            let radius = 23.0;
            painter.circle_stroke(avatar.center(), radius, Stroke::new(2.0, palette.green));
            let beat = ((now % 1.4) / 1.4) as f32;
            painter.circle_stroke(avatar.center(), radius * (1.0 + 0.45 * beat), Stroke::new(2.0, palette.green.gamma_multiply(1.0 - beat)));
        }
        let text_left = avatar.right() + 12.0;
        let text_width = origin.x + WIDTH - PAD - text_left;
        let used = crate::name_style::paint_name(ui, egui::pos2(text_left, top + 11.0), text_width, face.name, FontId::new(15.0, theme::semibold()), palette.strong, face.style);
        if face.live {
            let tag = painter.layout_no_wrap("AO VIVO".to_owned(), FontId::new(10.0, theme::bold()), Color32::WHITE);
            let rect = Rect::from_min_size(egui::pos2(text_left + used + 8.0, top + 4.0), tag.size() + Vec2::new(8.0, 2.0));
            painter.rect_filled(rect, CornerRadius::same(3), palette.danger);
            painter.galley(rect.min + Vec2::new(4.0, 1.0), tag, Color32::WHITE);
        }
        let body = painter.layout_job(body_job(face.text, face.mention.as_deref(), text_width, palette));
        let body_top = top + 22.0;
        let body_bottom = body_top + body.size().y;
        painter.galley(egui::pos2(text_left, body_top), body, palette.text);
        let mut bottom = avatar.bottom().max(body_bottom) + 12.0;

        // The answer: a field to reply, or the buttons.
        let tail_left = origin.x + PAD + INDENT;
        let tail_width = WIDTH - 2.0 * PAD - INDENT;
        match face.tail {
            Tail::Reply { draft, to } => {
                let field = Rect::from_min_size(egui::pos2(tail_left, bottom), Vec2::new(tail_width, 36.0));
                let fill = if palette.glass { Color32::from_black_alpha(56) } else { palette.rail };
                let edge_slot = painter.add(Shape::Noop);
                painter.rect_filled(field, CornerRadius::same(8), fill);
                let send_rect = Rect::from_center_size(egui::pos2(field.right() - 18.0, field.center().y), Vec2::splat(28.0));
                let edit_rect = Rect::from_min_max(egui::pos2(field.left() + 12.0, field.top()), egui::pos2(send_rect.left() - 4.0, field.bottom()));
                let hint = egui::RichText::new(format!("Responder a {to}")).color(palette.muted).size(14.0);
                let edit = ui.put(
                    edit_rect,
                    egui::TextEdit::singleline(draft).frame(egui::Frame::NONE).hint_text(hint).font(FontId::proportional(14.0)).text_color(palette.text).align(egui::Align2::LEFT_CENTER),
                );
                let send = ui.scope_builder(UiBuilder::new().max_rect(send_rect), |ui| widgets::icon_button(ui, icons::SEND, 28.0, palette.muted, palette.hover, "Enviar")).inner;
                if edit.has_focus() {
                    drawn.held = true;
                    painter.set(edge_slot, Shape::rect_stroke(field, CornerRadius::same(8), Stroke::new(1.0, palette.accent), egui::StrokeKind::Inside));
                }
                let entered = edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if send.clicked() || entered {
                    drawn.click = Some(Click::Send);
                }
                bottom = field.bottom();
            }
            Tail::Buttons(buttons) => {
                let count = buttons.len() as f32;
                let width = (tail_width - 8.0 * (count - 1.0)) / count;
                for (index, (icon, label, tone)) in buttons.into_iter().enumerate() {
                    let rect = Rect::from_min_size(egui::pos2(tail_left + index as f32 * (width + 8.0), bottom), Vec2::new(width, 34.0));
                    if action_button(ui, palette, id.with(index), rect, icon, label, tone).clicked() {
                        drawn.click = Some(Click::Button(index));
                    }
                }
                bottom += 34.0;
            }
        }
        bottom += 14.0;
        let rect = Rect::from_min_max(origin, egui::pos2(origin.x + WIDTH, bottom));
        drawn.height = rect.height();
        drawn.held |= ui.rect_contains_pointer(rect);

        if face.mention.is_some() {
            // The yellow mark of a mention, as the left edge of `.notice.mention`.
            let stripe = Rect::from_min_max(egui::pos2(rect.left() + 1.0, rect.top() + 10.0), egui::pos2(rect.left() + 4.0, rect.bottom() - 10.0));
            painter.rect_filled(stripe, CornerRadius::same(2), palette.yellow);
        }
        if let Some(time) = face.time {
            // What is left of the time, a thin line along the bottom that stops under the mouse.
            let full = Rect::from_min_max(egui::pos2(rect.left() + 8.0, rect.bottom() - 4.0), egui::pos2(rect.right() - 8.0, rect.bottom() - 1.0));
            let bar = Rect::from_min_size(full.min, Vec2::new(full.width() * time, full.height()));
            painter.rect_filled(bar, CornerRadius::same(2), palette.accent);
        }

        let corners = CornerRadius::same(RADIUS);
        match (look, reserved) {
            (Some(look), Some(reserved)) => {
                let shadow = egui::Shadow { offset: [0, 16], blur: 44, spread: 0, color: Color32::from_black_alpha(115) };
                let pane = crate::glass::Pane { shadow: Some(shadow), frost: true, ..crate::glass::Pane::new(look.side(), f32::from(RADIUS)) };
                crate::glass::fill(&painter, reserved, rect, &pane);
            }
            _ => {
                let alpha = if palette.dark { 115 } else { 36 };
                let shadow = egui::Shadow { offset: [0, 14], blur: 36, spread: 0, color: Color32::from_black_alpha(alpha) };
                painter.set(shadow_slot, shadow.as_shape(rect, corners));
                painter.set(
                    back_slot,
                    Shape::Vec(vec![Shape::rect_filled(rect, corners, palette.sidebar), Shape::rect_stroke(rect, corners, Stroke::new(1.0, palette.line_soft), egui::StrokeKind::Inside)]),
                );
            }
        }
        ui.allocate_rect(rect, Sense::hover());
    });
    drawn
}

/// The text of the card in two lines at most, with the mention of the person marked as in the
/// conversation.
fn body_job(text: &str, mention: Option<&str>, width: f32, palette: &Palette) -> LayoutJob {
    let plain = TextFormat { font_id: FontId::proportional(14.0), color: palette.text, line_height: Some(19.0), ..Default::default() };
    let marked = TextFormat {
        font_id: FontId::new(14.0, theme::semibold()),
        color: palette.accent_text,
        background: Color32::from_rgba_unmultiplied(88, 101, 242, 77),
        line_height: Some(19.0),
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    let lower = text.to_lowercase();
    let found = mention.and_then(|mention| lower.find(&mention.to_lowercase()).map(|at| (at, at + mention.len()))).filter(|(at, end)| text.is_char_boundary(*at) && text.is_char_boundary(*end));
    match found {
        Some((at, end)) => {
            job.append(&text[..at], 0.0, plain.clone());
            job.append(&text[at..end], 0.0, marked);
            job.append(&text[end..], 0.0, plain);
        }
        None => job.append(text, 0.0, plain),
    }
    job.wrap = TextWrapping { max_width: width, max_rows: 2, break_anywhere: false, overflow_character: Some('…') };
    job
}

fn action_button(ui: &mut Ui, palette: &Palette, id: Id, rect: Rect, icon: Icon, label: &str, tone: Tone) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let lit = crate::motion::hover(ui.ctx(), id, response.hovered());
    let painter = ui.painter();
    let corners = CornerRadius::same(8);
    match tone {
        Tone::Accent if palette.glass => {
            crate::glass::accent_fill(painter, Shape::rect_filled(rect, corners, Color32::WHITE));
            if lit > 0.0 {
                painter.rect_filled(rect, corners, Color32::from_white_alpha((18.0 * lit) as u8));
            }
        }
        _ => {
            let (rest, over) = match tone {
                Tone::Danger => (palette.danger, palette.danger_hover),
                Tone::Green => (palette.green, crate::motion::mix(palette.green, Color32::BLACK, 0.2)),
                Tone::Accent => (palette.accent, palette.accent_hover),
            };
            painter.rect_filled(rect, corners, crate::motion::mix(rest, over, lit));
        }
    }
    let text = painter.layout_no_wrap(label.to_owned(), FontId::new(14.0, theme::semibold()), Color32::WHITE);
    let total = 16.0 + 6.0 + text.size().x;
    let left = rect.center().x - total / 2.0;
    icon.image(16.0, Color32::WHITE).paint_at(ui, Rect::from_center_size(egui::pos2(left + 8.0, rect.center().y), Vec2::splat(16.0)));
    ui.painter().galley(egui::pos2(left + 22.0, rect.center().y - text.size().y / 2.0), text, Color32::WHITE);
    response
}
