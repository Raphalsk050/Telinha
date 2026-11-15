//! What calls for the person's attention: the count of messages not read, the notices of
//! Windows, the tunes of the rooms and the call that is ringing.

use std::collections::{HashMap, HashSet};

use egui::{Color32, CornerRadius, FontId, Pos2, Rect, RichText, Ui, Vec2};
use telinha_core::hub::{Conversation, DM_ROOM, OutgoingPhase};
use telinha_voice::tones::{self, Tune};

use crate::app::{Nav, TelinhaApp, first_text_channel};
use crate::dialogs::{Tone, button};
use crate::notify::Notices;
use crate::theme::{self, Palette};

/// A call rings again this often, and gives up after this long.
const RING_EVERY: f64 = 2.5;
const RING_FOR: f64 = 45.0;
/// Someone who stops watching and comes back within this time does not sound again.
const VIEWER_RETURN: f64 = 5.0;

struct Incoming {
    contact_id: String,
    name: String,
    live: bool,
    started: f64,
    rings: u32,
}

#[derive(Default)]
pub struct Alerts {
    /// Messages that arrived in conversations that were not open, by the key of each.
    unread: HashMap<String, u32>,
    notices: Option<Notices>,
    unfocused: bool,
    incoming: Option<Incoming>,
    /// The room, the people and the shares the tunes were last played for.
    room: Option<(String, String)>,
    people: HashSet<String>,
    sharing: HashSet<String>,
    was_live: bool,
    viewers: HashSet<String>,
    viewer_left: HashMap<String, f64>,
}

/// The red count at the right end of a row.
pub fn paint_badge(ui: &Ui, right_center: Pos2, count: u32, palette: &Palette) {
    let text = if count > 99 { "99+".to_owned() } else { count.to_string() };
    let galley = ui.painter().layout_no_wrap(text, FontId::new(11.0, theme::bold()), Color32::WHITE);
    let size = Vec2::new((galley.size().x + 8.0).max(16.0), 16.0);
    let rect = Rect::from_min_size(right_center - Vec2::new(size.x, size.y / 2.0), size);
    ui.painter().rect_filled(rect, CornerRadius::same(8), palette.danger);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

impl TelinhaApp {
    /// Lets the app show notices in Windows. The tests leave it off.
    pub fn enable_notices(&mut self) {
        self.alerts.notices = Some(Notices::new(self.hub.data_dir(), &self.context));
    }

    pub fn unread(&self, conversation: &Conversation) -> u32 {
        self.alerts.unread.get(&conversation.key()).copied().unwrap_or(0)
    }

    pub(crate) fn unread_in_server(&self, server_id: &str) -> u32 {
        let Some(server) = self.servers.iter().find(|server| server.id == server_id) else { return 0 };
        server.channels.iter().map(|channel| self.unread(&Conversation::channel(server_id, &channel.id))).sum()
    }

    pub(crate) fn unread_in_contacts(&self) -> u32 {
        self.contacts.iter().map(|(contact, _)| self.unread(&Conversation::direct(&contact.id))).sum()
    }

    pub(crate) fn mark_read(&mut self, conversation: &Conversation) {
        self.alerts.unread.remove(&conversation.key());
    }

    /// A message arrived. It counts as not read unless its conversation is the one open.
    pub(crate) fn count_unread(&mut self, conversation: &Conversation) {
        if self.conversation().as_ref() != Some(conversation) {
            *self.alerts.unread.entry(conversation.key()).or_insert(0) += 1;
        }
    }

    pub(crate) fn show_notice(&mut self, title: &str, body: &str, member_id: Option<&str>, open: Conversation) {
        if self.alerts.unfocused {
            self.context.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
        }
        let picture = member_id.and_then(|member| self.hub.picture(member, false));
        if let Some(notices) = &mut self.alerts.notices {
            // A notice Windows refuses is not worth telling the person about.
            let _ = notices.show(title, body, picture.as_ref(), open);
        }
    }

    pub(crate) fn open_conversation(&mut self, conversation: &Conversation) {
        let nav = match (&conversation.channel_id, self.servers.iter().find(|server| server.id == conversation.space_id)) {
            (Some(channel), Some(server)) => Nav::Server { server_id: server.id.clone(), channel_id: Some(channel.clone()) },
            (None, Some(server)) => Nav::Server { server_id: server.id.clone(), channel_id: first_text_channel(server) },
            (_, None) if self.contacts.iter().any(|(contact, _)| contact.id == conversation.space_id) => Nav::Direct(conversation.space_id.clone()),
            (_, None) => return,
        };
        self.navigate(nav);
    }

    /// Runs on every draw: tells the hub whether the person is looking, follows the click on a
    /// notice, and keeps the call ringing.
    pub(crate) fn tend_alerts(&mut self, ui: &Ui) {
        let unfocused = !ui.input(|input| input.viewport().focused.unwrap_or(true));
        if unfocused != self.alerts.unfocused {
            self.alerts.unfocused = unfocused;
            self.hub.set_window_focused(!unfocused);
        }
        let clicked = self.alerts.notices.as_ref().and_then(Notices::clicked);
        if let Some(conversation) = clicked {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
            self.open_conversation(&conversation);
        }

        let now = ui.input(|input| input.time);
        let deaf_in_a_room = self.room.voice.as_ref().is_some_and(|voice| voice.deaf);
        let mut ring = false;
        if let Some(incoming) = &mut self.alerts.incoming {
            let elapsed = now - incoming.started;
            if elapsed < RING_FOR {
                let due = f64::from(incoming.rings) * RING_EVERY;
                if elapsed >= due {
                    incoming.rings += 1;
                    ring = !deaf_in_a_room;
                }
                let wait = (f64::from(incoming.rings) * RING_EVERY - elapsed).max(0.05);
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait));
            }
        }
        if ring && self.room.audible() {
            tones::play(Tune::ring());
        }
    }

    fn tone(&self, tune: Tune) {
        let deaf = self.room.voice.as_ref().is_some_and(|voice| voice.deaf);
        if self.room.audible() && !deaf {
            tones::play(tune);
        }
    }

    pub(crate) fn incoming_call(&mut self, contact_id: String, name: String, live: bool) {
        if self.room.voice.as_ref().is_some_and(|voice| voice.space_id == contact_id) {
            return;
        }
        let started = self.context.input(|input| input.time);
        self.alerts.incoming = Some(Incoming { contact_id, name, live, started, rings: 0 });
    }

    /// Shows the card of a call without anyone calling, or takes it away with an empty name. For
    /// the tests.
    pub fn ring_for_test(&mut self, name: &str, live: bool) {
        let started = self.context.input(|input| input.time);
        self.alerts.incoming = (!name.is_empty()).then(|| Incoming { contact_id: String::new(), name: name.to_owned(), live, started, rings: 0 });
    }

    /// After the room, the people in it or the shares changed: plays the tune that tells it, and
    /// drops a ringing call that was answered or given up.
    pub(crate) fn after_room_change(&mut self) {
        let room = self.room.voice.as_ref().map(|voice| (voice.space_id.clone(), voice.room_id.clone()));
        let others: Vec<_> = match &room {
            Some((space_id, room_id)) => self.people_in(space_id, room_id).into_iter().filter(|person| !person.me).collect(),
            None => Vec::new(),
        };
        let people: HashSet<String> = others.iter().map(|person| person.id.clone()).collect();
        let sharing: HashSet<String> = others.iter().filter(|person| person.live).map(|person| person.id.clone()).collect();
        if room != self.alerts.room {
            // Leaving has to sound even though the person is, by then, out of the room.
            let tune = if room.is_some() { Tune::join() } else { Tune::leave() };
            if self.room.audible() {
                tones::play(tune);
            }
        } else if room.is_some() {
            if people.iter().any(|id| !self.alerts.people.contains(id)) {
                self.tone(Tune::join());
            } else if self.alerts.people.iter().any(|id| !people.contains(id)) {
                self.tone(Tune::leave());
            }
            if sharing.iter().any(|id| !self.alerts.sharing.contains(id)) {
                self.tone(Tune::live());
            }
        }
        self.alerts.room = room;
        self.alerts.people = people;
        self.alerts.sharing = sharing;

        let now = self.context.input(|input| input.time);
        let outgoing = self.room.streams.outgoing.as_ref();
        let live = outgoing.is_some_and(|outgoing| outgoing.phase == OutgoingPhase::Live);
        let viewers: HashSet<String> = outgoing.map(|outgoing| outgoing.viewers.iter().map(|(id, _)| id.clone()).collect()).unwrap_or_default();
        for gone in self.alerts.viewers.iter().filter(|id| !viewers.contains(*id)) {
            self.alerts.viewer_left.insert(gone.clone(), now);
        }
        let arrived = viewers
            .iter()
            .any(|id| !self.alerts.viewers.contains(id) && self.alerts.viewer_left.get(id).is_none_or(|left| now - left > VIEWER_RETURN));
        self.alerts.viewers = viewers;
        if arrived && live {
            self.tone(Tune::viewer());
        }
        if live && !self.alerts.was_live {
            self.tone(Tune::live());
        } else if !live && self.alerts.was_live {
            self.alerts.viewer_left.clear();
        }
        self.alerts.was_live = live;

        let over = self.alerts.incoming.as_ref().is_some_and(|incoming| {
            let answered = self.room.voice.as_ref().is_some_and(|voice| voice.space_id == incoming.contact_id);
            let calling = self.hub.peers(&incoming.contact_id).iter().any(|peer| peer.voice.as_ref().is_some_and(|voice| voice.room_id == DM_ROOM));
            answered || !calling
        });
        if over {
            self.alerts.incoming = None;
        }
    }

    fn answer_call(&mut self) {
        let Some(incoming) = self.alerts.incoming.take() else { return };
        self.navigate(Nav::Direct(incoming.contact_id.clone()));
        self.join_room(&incoming.contact_id, DM_ROOM);
        let joined = self.hub.voice().is_some_and(|voice| voice.space_id == incoming.contact_id);
        if incoming.live && joined {
            let sharer = self
                .hub
                .peers(&incoming.contact_id)
                .into_iter()
                .find(|peer| peer.voice.as_ref().is_some_and(|voice| voice.room_id == DM_ROOM && voice.live));
            if let Some(sharer) = sharer {
                if let Err(error) = self.hub.watch(&sharer.id) {
                    self.notice(error);
                }
            }
        }
    }

    /// A call coming in, as `#call-dialog` asked: a dialog in the middle of the window.
    pub(crate) fn show_incoming(&mut self, context: &egui::Context) {
        let Some(incoming) = &self.alerts.incoming else { return };
        let palette = self.palette;
        let (title, text, accept) = if incoming.live {
            (format!("{} está transmitindo a tela para você", incoming.name), "Entre na chamada para assistir e conversar.", "Entrar e assistir")
        } else {
            (format!("{} está chamando você", incoming.name), "Entre para conversar por voz, ver a câmera ou a tela.", "Entrar")
        };
        let mut answer = None;
        egui::Modal::new(egui::Id::unique("incoming-call"))
            .frame(crate::dialogs::dialog_frame(&palette))
            .backdrop_color(Color32::from_black_alpha(178))
            .show(context, |ui| {
                ui.set_width(420.0);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.add(egui::Label::new(RichText::new(title).color(palette.strong).size(18.0).family(theme::bold())).wrap());
                ui.add_space(15.0);
                ui.add(egui::Label::new(RichText::new(text).color(palette.muted).size(15.0)).wrap());
                ui.add_space(15.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if button(ui, &palette, accept, Tone::Primary).clicked() {
                        answer = Some(true);
                    }
                    if button(ui, &palette, "Agora não", Tone::Plain).clicked() {
                        answer = Some(false);
                    }
                });
            });
        match answer {
            Some(true) => self.answer_call(),
            Some(false) => self.alerts.incoming = None,
            None => {}
        }
    }
}
