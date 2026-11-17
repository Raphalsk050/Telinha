//! The voice room: who is in it, the screens being shared, and the buttons of the call.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Arc;

use egui::{Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};
use telinha_core::engine::Targets;
use telinha_core::hub::{
    DM_ROOM, EndReason, Hub, HubEvent, IncomingPhase, ManualRole, OutgoingPhase, SELF_VIEWER, StreamsView, VoiceState,
};
use telinha_core::servers::{Channel, Server};
use telinha_voice::camera::Camera;
use telinha_voice::video::{CameraFeed, FrameStore};
use telinha_voice::{Call, CallEvent, IceServer, Participant, PeerState, Sound, default_ice_servers};

use crate::app::{Nav, TelinhaApp};
use crate::dialogs::{Tone, button};
use crate::icons;
use crate::share_picker::Picker;
use crate::stream_settings::{AudioChoice, Quality, Shared};
use crate::theme::{self, Palette};
use crate::widgets;

const STAGE: Color32 = Color32::BLACK;
const LIVE_RED: Color32 = Color32::from_rgb(0xda, 0x37, 0x3c);

/// Someone in a voice room, the person here included.
#[derive(Debug, Clone)]
pub struct Person {
    /// The id of their app, which is what a stream is asked from.
    pub id: String,
    pub name: String,
    pub member_id: Option<String>,
    pub mic: bool,
    pub deaf: bool,
    pub live: bool,
    /// Whether their camera is on, as the presence says.
    pub camera: bool,
    pub me: bool,
}

/// Results of slow work asked from the window, delivered back to it.
pub enum Task {
    Targets(Result<Targets, String>),
    Call(CallEvent),
}

/// What the view over the whole window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ViewSource {
    /// A screen watched inside the window, by who shares it; the own one is `SELF_VIEWER`.
    Stream(String),
    OwnCamera,
    /// Someone's camera, by the id of their app.
    Camera(String),
}

/// The camera of this app while it is on in a call, and the room it was turned on in.
struct LiveCamera {
    /// None only in the tests, which show a picture in place of a camera.
    camera: Option<Camera>,
    space_id: String,
    room_id: String,
}

/// The call in progress and what is known about each connection in it.
struct ActiveCall {
    call: Call,
    space_id: String,
    room_id: String,
}

pub struct RoomState {
    pub voice: Option<VoiceState>,
    /// When the person entered the call, in the clock of the window.
    since: Option<f64>,
    pub streams: StreamsView,
    pub(crate) picker: Option<Picker>,
    pub(crate) share_tab: crate::share_picker::Tab,
    /// The quality and the sound of the last share, offered again the next time.
    pub(crate) last_share: Option<(Quality, AudioChoice)>,
    /// What the network of the share carries now, by the last report of its engine.
    pub(crate) share_network_kbps: Option<f64>,
    /// The sound inputs the picker found, for the sound of a capture card.
    pub(crate) audio_inputs: Vec<(String, String)>,
    pub(crate) sender: Sender<Task>,
    tasks: Receiver<Task>,
    call: Option<ActiveCall>,
    /// Where the sound of calls comes from. The tests swap the devices for a tone.
    pub sound: Sound,
    /// False in the tests that only draw the room: a real connection opens network ports, and
    /// Windows asks the person about the firewall for every new test program that does.
    connects: bool,
    peer_states: HashMap<String, PeerState>,
    speaking: HashSet<String>,
    /// The share that is running, in a call or with codes, for the window of its settings.
    pub(crate) shared: Option<Shared>,
    pub(crate) stream_settings: bool,
    pub(crate) stream_feedback: String,
    /// The tile that fills the stage after a click on it, by its id.
    focus: Option<String>,
    /// The line under the tiles, for what a button of the stage could not do.
    pub(crate) feedback: String,
    /// Where a drag of the handle of the stage started: the pointer and the height of the tiles.
    resizing: Option<(f32, f32)>,
    /// The balloon about the connection, above the call panel.
    pub(crate) popover: bool,
    pub(crate) popover_privacy: bool,
    pub(crate) popover_copied_at: Option<f64>,
    /// Where the call panel and its state button were last drawn, for the balloon to sit above.
    pub(crate) panel_rect: Option<Rect>,
    pub(crate) status_rect: Option<Rect>,
    pub(crate) sounds_rect: Option<Rect>,
    camera: Option<LiveCamera>,
    /// Where the frames of the camera go, made the first time it is turned on.
    pub(crate) camera_feed: Option<CameraFeed>,
    /// The pictures of the videos on the window, by whose video, with the number of the frame.
    textures: HashMap<String, (u64, egui::TextureHandle)>,
    /// The screens opened inside the call, by who shares them.
    stream_views: HashSet<String>,
    /// The video shown over the whole window, with its name.
    pub(crate) viewer: Option<(ViewSource, String)>,
    /// When the next small picture of the own share goes to the room.
    thumb_due: Option<f64>,
    /// Pictures that stand for screens in the tests, which have no call.
    test_streams: HashMap<String, Arc<FrameStore>>,
}

impl RoomState {
    pub fn new(hub: &Hub) -> Self {
        let (sender, tasks) = channel();
        Self {
            voice: hub.voice(),
            since: None,
            streams: hub.streams(),
            picker: None,
            share_tab: crate::share_picker::Tab::default(),
            last_share: None,
            share_network_kbps: None,
            audio_inputs: Vec::new(),
            sender,
            tasks,
            call: None,
            sound: Sound::Devices,
            connects: true,
            peer_states: HashMap::new(),
            speaking: HashSet::new(),
            shared: None,
            stream_settings: false,
            stream_feedback: String::new(),
            focus: None,
            feedback: String::new(),
            resizing: None,
            popover: false,
            popover_privacy: false,
            popover_copied_at: None,
            panel_rect: None,
            status_rect: None,
            sounds_rect: None,
            camera: None,
            camera_feed: None,
            textures: HashMap::new(),
            stream_views: HashSet::new(),
            viewer: None,
            thumb_due: None,
            test_streams: HashMap::new(),
        }
    }
}

impl RoomState {
    /// Drops the pictures of the videos, for a window that was just made and does not have them.
    pub(crate) fn forget_textures(&mut self) {
        self.textures.clear();
    }

    /// Whether the app may make sound on the speakers. Not in the tests.
    pub(crate) fn audible(&self) -> bool {
        self.connects && matches!(self.sound, Sound::Devices)
    }

    /// Fires a sound of the soundboard into the call. False when there is no call to hear it.
    pub(crate) fn play_sound(&self, samples: Arc<[i16]>, volume: f32) -> bool {
        match &self.call {
            Some(call) => {
                call.call.play_sound(samples, volume);
                true
            }
            None => false,
        }
    }

    pub(crate) fn peer_state(&self, id: &str) -> Option<PeerState> {
        self.peer_states.get(id).copied()
    }

    pub(crate) fn picker_is_manual(&self) -> bool {
        self.picker.as_ref().is_some_and(|picker| picker.manual)
    }

    pub(crate) fn close_picker(&mut self) {
        self.picker = None;
    }
}

/// `1:23:00` past the hour, `4:05` before it.
pub fn format_elapsed(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

impl TelinhaApp {
    pub(crate) fn room_event(&mut self, event: HubEvent) {
        match event {
            HubEvent::Voice => {
                let voice = self.hub.voice();
                let moved = voice.as_ref().map(|voice| (&voice.space_id, &voice.room_id))
                    != self.room.voice.as_ref().map(|voice| (&voice.space_id, &voice.room_id));
                if moved {
                    self.room.since = voice.is_some().then(|| self.context.input(|input| input.time));
                    self.room.focus = None;
                    self.room.feedback.clear();
                    if voice.is_none() {
                        self.room.popover = false;
                    }
                }
                if let (Some(call), Some(voice)) = (&self.room.call, &voice) {
                    // As in Discord, deafening also silences the microphone; hearing again
                    // brings it back as it was.
                    call.call.set_mic(voice.mic && !voice.deaf);
                    call.call.set_deaf(voice.deaf);
                }
                self.room.voice = voice;
                self.sync_call();
            }
            HubEvent::Streams => {
                self.room.streams = self.hub.streams();
                self.sync_stream_views();
            }
            HubEvent::EmbeddedOffer { sharer_id, sdp } => {
                if let Some(call) = &self.room.call {
                    call.call.open_stream(&sharer_id, &sdp);
                    if sharer_id != SELF_VIEWER {
                        call.call.set_stream_volume(&sharer_id, self.stream_volume_of(&sharer_id));
                    }
                    self.room.stream_views.insert(sharer_id);
                }
            }
            HubEvent::Peers { space_id } => {
                if self.room.call.as_ref().is_some_and(|call| call.space_id == space_id) {
                    self.sync_participants();
                }
            }
            HubEvent::Rtc { space_id, from, payload } => {
                if let Some(call) = self.room.call.as_ref().filter(|call| call.space_id == space_id) {
                    call.call.handle_rtc(&from, payload);
                }
            }
            HubEvent::EngineEvent { source, event } => {
                self.debug_engine_event(&source, &event);
                if source == "outgoing" {
                    self.stream_feedback(&event);
                }
            }
            HubEvent::OutgoingEnded { code, stopped, error } => {
                self.debug.remove("outgoing");
                self.room.picker = None;
                if self.room.shared.is_some_and(|shared| !shared.manual) {
                    self.room.shared = None;
                }
                let stage = error.as_ref().and_then(|error| error.get("stage")).and_then(|stage| stage.as_str());
                if stage == Some("target_closed") {
                    self.notice("A janela que você estava transmitindo foi fechada, então a transmissão parou.");
                } else if let Some(error) = error {
                    let message = error.get("message").and_then(|message| message.as_str()).unwrap_or("erro desconhecido");
                    self.notice(format!("A transmissão parou com erro: {message}."));
                } else if !stopped && code != Some(0) {
                    self.notice(format!("A transmissão parou sozinha (código {}).", code.map_or_else(|| "?".to_owned(), |code| code.to_string())));
                }
            }
            HubEvent::IncomingEnded { sharer_id, name, reason, error } => {
                self.debug.remove(&sharer_id);
                self.incoming_ended(&name, reason, error.as_ref());
            }
            _ => {}
        }
    }

    fn incoming_ended(&mut self, name: &str, reason: EndReason, error: Option<&serde_json::Value>) {
        match reason {
            EndReason::Failed if error.and_then(|error| error.get("stage")).and_then(|stage| stage.as_str()) == Some("timeout") => {
                self.notice(format!("{name} não respondeu a tempo. Tente assistir de novo."));
            }
            EndReason::Failed => self.notice(format!("Não consegui assistir {name}.")),
            EndReason::Ended => self.notice(format!("{name} parou de transmitir.")),
            EndReason::Stopped | EndReason::Closed => {}
        }
    }

    pub(crate) fn drain_tasks(&mut self) {
        self.send_thumb();
        let failed = self.room.camera.as_ref().and_then(|live| live.camera.as_ref()?.problem());
        if let Some(problem) = failed {
            self.set_camera(false);
            self.notice(problem);
        }
        while let Ok(task) = self.room.tasks.try_recv() {
            match task {
                Task::Targets(result) => {
                    if let Some(picker) = &mut self.room.picker {
                        picker.receive(result);
                    }
                }
                Task::Call(event) => match event {
                    CallEvent::Peer { id, state } => {
                        self.room.peer_states.insert(id, state);
                    }
                    CallEvent::Speaking { id, speaking } => {
                        if speaking {
                            self.room.speaking.insert(id);
                        } else {
                            self.room.speaking.remove(&id);
                        }
                    }
                    CallEvent::Report(line) => {
                        self.hub.log().info(&format!("chamada: {line}"));
                        self.debug.log(format!("[chamada] {line}"));
                    }
                    CallEvent::Problem(text) => self.notice(format!("Chamada: {text}.")),
                    CallEvent::Stats(stats) => self.call_stats(stats),
                    CallEvent::StreamAnswer { sharer_id, sdp } => {
                        if self.room.stream_views.contains(&sharer_id) {
                            self.hub.answer_embedded(&sharer_id, &sdp);
                        }
                    }
                    CallEvent::StreamState { sharer_id, live } => {
                        if sharer_id != SELF_VIEWER {
                            self.hub.embedded_state(&sharer_id, live);
                        }
                    }
                    CallEvent::StreamStats { sharer_id, stats } => {
                        if self.room.stream_views.contains(&sharer_id) {
                            self.stream_view_stats(&sharer_id, stats);
                        }
                    }
                },
            }
        }
    }

    /// Everyone in a voice room, the person here first when they are in it.
    pub(crate) fn people_in(&self, space_id: &str, room_id: &str) -> Vec<Person> {
        let mut people = Vec::new();
        if let Some(voice) = self.room.voice.as_ref().filter(|voice| voice.space_id == space_id && voice.room_id == room_id) {
            let live = self.room.streams.outgoing.as_ref().is_some_and(|outgoing| outgoing.phase == OutgoingPhase::Live);
            people.push(Person {
                id: self.hub.instance_id().to_owned(),
                name: self.hub.profile_name(),
                member_id: Some(self.hub.member_id()),
                mic: voice.mic,
                deaf: voice.deaf,
                live,
                camera: self.camera_on(),
                me: true,
            });
        }
        let contact_name = self.contacts.iter().find(|(contact, _)| contact.id == space_id).map(|(contact, _)| contact.name.clone());
        for peer in self.hub.peers(space_id) {
            let Some(voice) = peer.voice.as_ref().filter(|voice| voice.room_id == room_id) else { continue };
            let name = contact_name.clone().unwrap_or_else(|| if peer.name.is_empty() { "Alguém".to_owned() } else { peer.name.clone() });
            people.push(Person { id: peer.id.clone(), name, member_id: peer.member_id.clone(), mic: voice.mic, deaf: voice.deaf, live: voice.live, camera: voice.camera, me: false });
        }
        people
    }

    /// Closes the screens of the call that are no longer watched inside the window, as
    /// `StreamView.sync`: the screen of this app stays while it is shared.
    fn sync_stream_views(&mut self) {
        let mut wanted: HashSet<String> = self.room.streams.incoming.iter().filter(|entry| entry.embedded).map(|entry| entry.sharer_id.clone()).collect();
        if self.room.streams.outgoing.is_some() {
            wanted.insert(SELF_VIEWER.to_owned());
        }
        let gone: Vec<String> = self.room.stream_views.iter().filter(|id| !wanted.contains(*id)).cloned().collect();
        for id in gone {
            if let Some(call) = &self.room.call {
                call.call.close_stream(&id);
            }
            self.room.stream_views.remove(&id);
            self.room.textures.remove(&format!("stream:{id}"));
            self.debug.remove(&format!("watch-{id}"));
            if self.room.viewer.as_ref().is_some_and(|(viewer, _)| viewer == &ViewSource::Stream(id.clone())) {
                self.close_viewer();
            }
        }
    }

    /// Sends a small picture of the own share to the room now and then, for who has not opened
    /// it yet, as `sendThumb`: every 20 seconds, the first soon after going live.
    fn send_thumb(&mut self) {
        if !self.outgoing_live() {
            self.room.thumb_due = None;
            return;
        }
        let now = self.context.input(|input| input.time);
        let due = *self.room.thumb_due.get_or_insert(now + 1.5);
        if now < due {
            self.context.request_repaint_after(std::time::Duration::from_secs_f64(due - now));
            return;
        }
        self.room.thumb_due = Some(now + 20.0);
        let Some(voice) = self.room.voice.clone() else { return };
        if self.people_in(&voice.space_id, &voice.room_id).len() < 2 {
            return;
        }
        let Some((_, picture)) = self.stream_store(SELF_VIEWER).and_then(|store| store.latest()) else { return };
        let hub = Arc::clone(&self.hub);
        self.runtime.spawn_blocking(move || {
            if let Some(image) = thumb_of(&picture) {
                hub.publish_stream_thumb(&image);
            }
        });
    }

    /// How loud the screen of someone plays, in percent, as kept by the person here.
    fn stream_volume_of(&self, sharer_id: &str) -> f32 {
        let space = self.room.voice.as_ref().map(|voice| voice.space_id.clone()).unwrap_or_default();
        let key = self.hub.peers(&space).into_iter().find(|peer| peer.id == sharer_id).and_then(|peer| peer.member_id).unwrap_or_else(|| sharer_id.to_owned());
        self.prefs.stream_volumes.get(&key).copied().unwrap_or(100.0)
    }

    /// The picture of a screen watched inside the call.
    fn stream_store(&self, sharer_id: &str) -> Option<Arc<FrameStore>> {
        match &self.room.call {
            Some(call) => call.call.stream_video(sharer_id),
            None => self.room.test_streams.get(sharer_id).cloned(),
        }
    }

    /// Shows a picture as a screen watched inside the window. For the tests, which have no call:
    /// the store it returns takes more frames, as a decoder would.
    pub fn stream_for_test(&mut self, sharer_id: &str, name: &str, width: u32, height: u32, rgba: Vec<u8>) -> Option<Arc<FrameStore>> {
        let voice = self.room.voice.clone()?;
        let context = self.context.clone();
        let store = FrameStore::new(video_waker(context));
        store.put(telinha_voice::video::Picture { width, height, rgba });
        self.room.test_streams.insert(sharer_id.to_owned(), Arc::clone(&store));
        self.room.streams.incoming.retain(|entry| entry.sharer_id != sharer_id);
        self.room.streams.incoming.push(telinha_core::hub::IncomingView {
            sharer_id: sharer_id.to_owned(),
            space_id: voice.space_id,
            room_id: voice.room_id,
            name: name.to_owned(),
            phase: IncomingPhase::Live,
            fullscreen: false,
            embedded: true,
        });
        Some(store)
    }

    /// Shows a screen over the whole window, as `openStreamViewer`.
    pub fn open_viewer(&mut self, sharer_id: &str, label: String) {
        self.open_tile_viewer(ViewSource::Stream(sharer_id.to_owned()), label);
    }

    /// Shows a video over the whole window, in full screen, as `openTileViewer`.
    pub(crate) fn open_tile_viewer(&mut self, source: ViewSource, label: String) {
        self.room.viewer = Some((source, label));
        self.context.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
    }

    pub fn close_viewer(&mut self) {
        if self.room.viewer.take().is_some() {
            self.context.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
    }

    /// The screen over the whole window. A click or Esc closes it.
    pub(crate) fn show_viewer(&mut self, context: &egui::Context) {
        let Some((source, label)) = self.room.viewer.clone() else { return };
        let (key, store, mirror) = match &source {
            ViewSource::Stream(id) => (format!("stream:{id}"), self.stream_store(id), false),
            ViewSource::OwnCamera => (SELF_VIDEO.to_owned(), self.room.camera_feed.as_ref().filter(|_| self.camera_on()).map(|feed| Arc::clone(feed.preview())), true),
            ViewSource::Camera(id) => (id.clone(), self.room.call.as_ref().and_then(|call| call.call.video_of(id)), false),
        };
        let Some(store) = store else {
            self.close_viewer();
            return;
        };
        let screen = context.content_rect();
        let texture = self.video_texture(context, &key, &store, screen.size());
        // The person whose video it is, for the menu on the right button, as on their tile.
        let owner = match &source {
            ViewSource::Stream(id) | ViewSource::Camera(id) if id != SELF_VIEWER => self
                .room
                .voice
                .clone()
                .and_then(|voice| self.people_in(&voice.space_id, &voice.room_id).into_iter().find(|person| &person.id == id).map(|person| (person, voice.space_id))),
            _ => None,
        };
        // A click that only closes an open menu leaves the video where it is.
        let menu_open = egui::Popup::is_any_open(context);
        let mut close = context.input(|input| input.key_pressed(egui::Key::Escape)) && !menu_open;
        egui::Area::new(egui::Id::unique("tile-viewer")).order(egui::Order::Foreground).fixed_pos(screen.min).show(context, |ui| {
            let (rect, response) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(rect, CornerRadius::ZERO, Color32::BLACK);
            if let Some(texture) = texture {
                let size = texture.size;
                let scale = (rect.width() / size.x).min(rect.height() / size.y);
                let place = Rect::from_center_size(rect.center(), size * scale);
                let uv = if mirror { Rect::from_min_max(egui::pos2(1.0, 0.0), egui::pos2(0.0, 1.0)) } else { Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)) };
                ui.painter().image(texture.id, place, uv, Color32::WHITE);
            }
            // As `.tile-viewer .tile-label`: 16 from the corner, a little larger.
            tile_label(ui, rect.shrink(8.0), &label, None, 15.0 / 13.0);
            close |= response.clicked() && !menu_open;
            if let Some((person, space_id)) = &owner {
                self.person_menu(&response, person, space_id, true);
            }
        });
        if close {
            self.close_viewer();
        }
    }

    /// Feeds the debug panel and the balloon with the numbers of the call, as `handleCallStats`.
    fn call_stats(&mut self, stats: telinha_voice::CallStats) {
        let Some(voice) = self.room.voice.clone() else { return };
        let finite = |value: Option<f64>, digits: usize, unit: &str| value.filter(|value| value.is_finite()).map_or_else(|| "—".to_owned(), |value| format!("{}{unit}", format!("{value:.digits$}").replace('.', ",")));
        let percent = |value: Option<f64>| value.map(|value| value * 100.0);
        let rows = vec![
            ("Conectados", format!("{} de {}", stats.connected, stats.peers)),
            ("Ping médio", finite(stats.rtt, 0, " ms")),
            ("Perda de pacotes", finite(percent(stats.loss), 2, " %")),
            ("Voz reconstruída", finite(percent(stats.concealment), 1, " %")),
            ("Voz esticada", finite(percent(stats.stretch), 1, " %")),
            ("Buffer da voz", finite(stats.buffer, 0, " ms")),
            ("Microfone", if voice.mic { "ligado" } else { "mudo" }.to_owned()),
            ("Câmera", if self.camera_on() { "ligada" } else { "desligada" }.to_owned()),
        ];
        let summary = format!("{} · voz e câmera direto entre os computadores", self.place_name(&voice));
        self.debug.update("call", "Chamada", &summary, stats.rtt, percent(stats.loss), rows);
    }

    /// Feeds the debug panel with the numbers of a screen watched inside the window, as
    /// `handleStreamViewStats`.
    fn stream_view_stats(&mut self, sharer_id: &str, stats: telinha_voice::StreamStats) {
        let finite = |value: Option<f64>, digits: usize, unit: &str| value.filter(|value| value.is_finite()).map_or_else(|| "—".to_owned(), |value| format!("{}{unit}", format!("{value:.digits$}").replace('.', ",")));
        let loss = stats.loss.map(|loss| loss * 100.0);
        let rows = vec![
            ("Quadros por segundo", finite(stats.fps, 0, "")),
            ("Resolução", stats.size.map_or_else(|| "—".to_owned(), |(width, height)| format!("{width}×{height}"))),
            ("Ping", finite(stats.rtt, 0, " ms")),
            ("Perda de pacotes", finite(loss, 2, " %")),
            ("Taxa recebida", finite(stats.bitrate.map(|kbps| kbps / 1000.0), 1, " Mbps")),
            ("Atraso do buffer", finite(stats.buffer, 0, " ms")),
            ("Quadros descartados", stats.dropped.to_string()),
            ("Decodificador", if stats.decoder.is_empty() { "—".to_owned() } else { stats.decoder }),
        ];
        let name = self.room.streams.incoming.iter().find(|entry| entry.sharer_id == sharer_id).map_or("alguém", |entry| entry.name.as_str()).to_owned();
        let summary = format!("Recebendo a tela de {name} direto do computador dessa pessoa, dentro do app.");
        self.debug.update(&format!("watch-{sharer_id}"), &format!("Assistindo {name}"), &summary, stats.rtt, loss, rows);
    }

    /// Starts, moves or ends the voice connections to match the room the person is in.
    fn sync_call(&mut self) {
        let wanted = self.room.voice.as_ref().map(|voice| (voice.space_id.clone(), voice.room_id.clone()));
        let current = self.room.call.as_ref().map(|call| (call.space_id.clone(), call.room_id.clone()));
        if wanted == current {
            return;
        }
        if let Some(call) = self.room.call.take() {
            call.call.stop();
        }
        if let Some(feed) = &self.room.camera_feed {
            feed.set_call(None);
        }
        // The camera stays on only in the room it was turned on in, as call.js turned it off on
        // leaving.
        if self.room.camera.as_ref().is_some_and(|live| wanted.as_ref() != Some(&(live.space_id.clone(), live.room_id.clone()))) {
            self.set_camera(false);
        }
        self.room.textures.retain(|id, _| id == SELF_VIDEO);
        self.room.stream_views.clear();
        self.close_viewer();
        self.room.peer_states.clear();
        self.room.speaking.clear();
        self.debug.remove("call");
        let (Some((space_id, room_id)), Some(voice)) = (wanted, self.room.voice.clone()) else { return };
        if !self.room.connects {
            return;
        }

        let mut ice_servers = default_ice_servers();
        if let Some(relay) = self.servers.iter().find(|server| server.id == space_id).and_then(|server| server.relay.clone()) {
            ice_servers.push(IceServer { urls: relay.urls, username: relay.username, password: relay.credential });
        }
        let hub = Arc::clone(&self.hub);
        let space = space_id.clone();
        let _runtime = self.runtime.enter();
        let (call, mut events) = Call::start(self.hub.instance_id(), ice_servers, self.room.sound, voice.mic && !voice.deaf, voice.deaf, move |to, payload| {
            hub.send_rtc(&space, to, &payload);
        });
        let context = self.context.clone();
        call.set_waker(video_waker(context));
        if let Some(feed) = &self.room.camera_feed {
            feed.set_call(Some(&call));
        }
        let sender = self.room.sender.clone();
        let context = self.context.clone();
        self.runtime.spawn(async move {
            while let Some(event) = events.recv().await {
                if sender.send(Task::Call(event)).is_err() {
                    return;
                }
                context.request_repaint();
            }
        });
        self.room.call = Some(ActiveCall { call, space_id, room_id });
        self.sync_participants();
    }

    /// Tells the call who is in the room now, so it connects to who arrived and drops who left.
    fn sync_participants(&mut self) {
        let Some(call) = &self.room.call else { return };
        let peers: Vec<_> = self.hub.peers(&call.space_id).into_iter().filter(|peer| peer.voice.as_ref().is_some_and(|voice| voice.room_id == call.room_id)).collect();
        let participants = peers
            .iter()
            .filter_map(|peer| Some(Participant { answers_repeats: peer.voice.as_ref()?.version >= 2, name: peer.name.clone(), id: peer.id.clone() }))
            .collect();
        call.call.set_participants(participants);
        // Each person comes back as loud as they were left the last time, or silenced.
        for peer in &peers {
            let key = peer.member_id.as_deref().unwrap_or(&peer.id);
            if self.prefs.volumes.contains_key(key) || self.prefs.muted.contains(key) {
                call.call.set_volume(&peer.id, self.call_volume(key));
            }
        }
    }

    /// Ends the voice connections and opens them again, for a change of microphone or speakers
    /// to take hold.
    pub(crate) fn restart_call(&mut self) {
        if let Some(call) = self.room.call.take() {
            call.call.stop();
            self.sync_call();
            // The screens watched inside the old call went with it; they are asked for again.
            let embedded: Vec<String> = self.room.streams.incoming.iter().filter(|entry| entry.embedded).map(|entry| entry.sharer_id.clone()).collect();
            for sharer_id in embedded {
                self.hub.unwatch(&sharer_id);
                if let Err(error) = self.hub.watch(&sharer_id) {
                    self.notice(format!("Não consegui assistir: {error}."));
                }
            }
        }
    }

    /// How loud someone is played in the call: nothing while silenced here.
    fn call_volume(&self, key: &str) -> f32 {
        if self.prefs.muted.contains(key) { 0.0 } else { self.prefs.volumes.get(key).copied().unwrap_or(100.0) }
    }

    /// The menu of someone, as `openPersonMenu` built it: talk to them, how loud they are played
    /// here while in the same room, and their id.
    pub(crate) fn person_menu(&mut self, response: &egui::Response, person: &Person, space_id: &str, in_room: bool) {
        enum Pick {
            Message(String),
            Call(String),
            Contact(String, String),
            CopyId(String),
        }
        let palette = self.palette;
        let contact = self.contacts.iter().find(|(contact, _)| contact.id == space_id).map(|(contact, _)| (contact.id.clone(), contact.name.clone()));
        let calling = self.room.voice.as_ref().is_some_and(|voice| voice.space_id == space_id);
        let in_room = in_room && !person.me;
        let key = person.member_id.clone().unwrap_or_else(|| person.id.clone());
        let mut volume = self.prefs.volumes.get(&key).copied().unwrap_or(100.0);
        let mut stream_volume = self.prefs.stream_volumes.get(&key).copied().unwrap_or(100.0);
        let muted = self.prefs.muted.contains(&key);
        let hidden = self.prefs.video_hidden.contains(&key);
        let mut toggle_video = false;
        let streaming = self.room.streams.incoming.iter().any(|entry| entry.sharer_id == person.id);
        let header = if person.me { format!("{} (você)", person.name) } else { person.name.clone() };
        let (mut pick, mut volume_changed, mut stream_changed, mut settled, mut toggle_mute) = (None, false, false, false, false);
        crate::app::context_menu(response, &palette, |ui| {
            crate::app::menu_header(ui, &palette, &header);
            if let Some((id, _)) = &contact {
                if crate::app::menu_item(ui, &palette, "Mensagem", false) {
                    pick = Some(Pick::Message(id.clone()));
                }
                if !calling && crate::app::menu_item(ui, &palette, "Iniciar chamada", false) {
                    pick = Some(Pick::Call(id.clone()));
                }
            }
            if in_room {
                crate::app::menu_separator(ui, &palette);
                let slider = crate::app::menu_slider(ui, &palette, "Volume do usuário", &mut volume, 0.0..=200.0);
                volume_changed = slider.changed();
                settled |= slider.drag_stopped() || (slider.changed() && !slider.dragged());
                if streaming {
                    let slider = crate::app::menu_slider(ui, &palette, "Volume da transmissão", &mut stream_volume, 0.0..=200.0);
                    stream_changed = slider.changed();
                    settled |= slider.drag_stopped() || (slider.changed() && !slider.dragged());
                }
                toggle_mute = crate::app::menu_check(ui, &palette, "Silenciar", muted);
                toggle_video = crate::app::menu_check(ui, &palette, "Desativar vídeo", hidden);
            }
            if let Some((id, name)) = &contact {
                crate::app::menu_separator(ui, &palette);
                if crate::app::menu_item(ui, &palette, "Renomear ou remover contato", false) {
                    pick = Some(Pick::Contact(id.clone(), name.clone()));
                }
            }
            if let Some(member_id) = &person.member_id {
                crate::app::menu_separator(ui, &palette);
                if crate::app::menu_item(ui, &palette, "Copiar ID do usuário", false) {
                    pick = Some(Pick::CopyId(member_id.clone()));
                }
            }
        });
        if volume_changed {
            let volume = volume.round();
            if volume == 100.0 {
                self.prefs.volumes.remove(&key);
            } else {
                self.prefs.volumes.insert(key.clone(), volume);
            }
        }
        if stream_changed {
            let stream_volume = stream_volume.round();
            if stream_volume == 100.0 {
                self.prefs.stream_volumes.remove(&key);
            } else {
                self.prefs.stream_volumes.insert(key.clone(), stream_volume);
            }
            self.hub.set_stream_volume(&person.id, stream_volume as i64);
            if let Some(call) = &self.room.call {
                call.call.set_stream_volume(&person.id, stream_volume);
            }
        }
        if toggle_mute {
            if muted {
                self.prefs.muted.remove(&key);
            } else {
                self.prefs.muted.insert(key.clone());
            }
            settled = true;
        }
        if toggle_video {
            if hidden {
                self.prefs.video_hidden.remove(&key);
            } else {
                self.prefs.video_hidden.insert(key.clone());
            }
            settled = true;
        }
        if volume_changed || toggle_mute {
            let volume = self.call_volume(&key);
            if let Some(call) = &self.room.call {
                call.call.set_volume(&person.id, volume);
            }
        }
        if settled {
            self.prefs.save();
        }
        match pick {
            Some(Pick::Message(id)) => self.navigate(Nav::Direct(id)),
            Some(Pick::Call(id)) => {
                self.navigate(Nav::Direct(id.clone()));
                self.join_room(&id, DM_ROOM);
            }
            Some(Pick::Contact(id, name)) => {
                self.dialog = Some(crate::dialogs::Dialog::for_purpose(crate::dialogs::Purpose::Contact(id), &name));
            }
            Some(Pick::CopyId(id)) => response.ctx.copy_text(id),
            None => {}
        }
    }

    /// Swaps the sound devices for a tone. For the tests, before entering a call.
    pub fn set_call_sound(&mut self, sound: Sound) {
        self.room.sound = sound;
    }

    /// Makes one tile of the stage fill it, as a click on the tile does. For the tests.
    pub fn focus_tile(&mut self, id: Option<&str>) {
        self.room.focus = id.map(str::to_owned);
    }

    /// Draws rooms without connecting to anyone in them. For the tests.
    pub fn disable_calls(&mut self) {
        self.room.connects = false;
    }

    /// Whether the connection to someone is up, for the tile to say so.
    pub(crate) fn peer_note(&self, id: &str) -> Option<&'static str> {
        match self.room.peer_states.get(id) {
            Some(PeerState::Connected) => None,
            Some(PeerState::Failed) => Some("reconectando…"),
            Some(PeerState::Connecting) | None => Some("conectando…"),
        }
    }

    pub(crate) fn is_speaking(&self, id: &str) -> bool {
        self.room.speaking.contains(id)
    }

    fn in_room(&self, space_id: &str, room_id: &str) -> bool {
        self.room.voice.as_ref().is_some_and(|voice| voice.space_id == space_id && voice.room_id == room_id)
    }

    pub(crate) fn join_room(&mut self, space_id: &str, room_id: &str) {
        if self.hub.join_voice(space_id, room_id, true, false, false).is_none() {
            self.notice("Essa sala não existe mais.");
        }
    }

    /// The people of a voice channel, listed under it in the sidebar.
    pub(crate) fn voice_rows(&mut self, ui: &mut Ui, server: &Server, channel: &Channel) {
        let palette = self.side;
        let people = self.people_in(&server.id, &channel.id);
        if people.is_empty() {
            return;
        }
        // As `.voice-people`: rows of 30 pixels, 30 in from the channels, 4 below the list.
        let spacing = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = 0.0;
        for person in people {
            let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
            let in_room = self.room.voice.as_ref().is_some_and(|voice| voice.space_id == server.id && voice.room_id == channel.id);
            self.person_menu(&response, &person, &server.id, in_room);
            let avatar = Rect::from_center_size(rect.left_center() + Vec2::new(30.0 + 6.0 + 12.0, 0.0), Vec2::splat(24.0));
            let picture = self.pictures.avatar(&self.hub, person.member_id.as_deref());
            widgets::paint_avatar(ui, avatar, &person.name, picture);
            // Who is speaking gets a green ring, as `.voice-person.speaking .avatar`.
            if self.is_speaking(&person.id) {
                ui.painter().circle_stroke(avatar.center(), 13.0, Stroke::new(2.0, palette.green));
            }
            // The badge and the icons sit at the right, 8 apart, as the flex row laid them.
            let mut right = rect.right() - 6.0;
            if person.deaf || !person.mic {
                let icon = if person.deaf { icons::DEAF } else { icons::MIC_OFF };
                // Red, as `.muted-icon { color: var(--danger) }`.
                icon.image(16.0, palette.danger).paint_at(ui, Rect::from_min_size(egui::pos2(right - 16.0, rect.center().y - 8.0), Vec2::splat(16.0)));
                right -= 16.0 + 8.0;
            }
            if person.live {
                let galley = ui.painter().layout_no_wrap("AO VIVO".to_owned(), FontId::new(9.0, theme::bold()), Color32::WHITE);
                let size = Vec2::new(galley.size().x + 8.0, galley.size().y.max(12.0));
                let badge = Rect::from_min_size(egui::pos2(right - size.x, rect.center().y - size.y / 2.0), size);
                ui.painter().rect_filled(badge, CornerRadius::same(4), palette.danger);
                ui.painter().galley(badge.center() - galley.size() / 2.0, galley, Color32::WHITE);
                right -= size.x + 8.0;
            }
            let left = avatar.right() + 8.0;
            widgets::paint_text_clipped(ui, egui::pos2(left, rect.center().y), right - left, &person.name, FontId::proportional(14.0), palette.muted);
        }
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = spacing;
    }

    /// The time in the call, for the row of the channel the person is in.
    pub(crate) fn room_timer(&self, space_id: &str, room_id: &str) -> Option<String> {
        if !self.in_room(space_id, room_id) {
            return None;
        }
        let now = self.context.input(|input| input.time);
        self.context.request_repaint_after(std::time::Duration::from_millis(500));
        Some(format_elapsed(now - self.room.since.unwrap_or(now)))
    }

    /// Whether the panel of the call shows over the strip of the person: in a call, or while a
    /// connection with codes runs.
    pub(crate) fn call_panel_shown(&self) -> bool {
        self.room.voice.is_some() || self.manual.as_ref().is_some_and(|manual| manual.active())
    }

    /// Where the call is, for the second line of the call panel.
    pub(crate) fn place_name(&self, voice: &VoiceState) -> String {
        if let Some((contact, _)) = self.contacts.iter().find(|(contact, _)| contact.id == voice.space_id) {
            return format!("Chamada com {}", contact.name);
        }
        match self.servers.iter().find(|server| server.id == voice.space_id) {
            Some(server) => {
                let room = server.channels.iter().find(|channel| channel.id == voice.room_id).map_or("Sala", |channel| channel.name.as_str());
                format!("{room} / {}", server.name)
            }
            None => "Chamada".to_owned(),
        }
    }

    /// The panel above the strip of the person while in a call or in a connection with codes,
    /// as `.call-panel` drew it: the state, where, the way out and a row of buttons.
    pub(crate) fn voice_panel(&mut self, ui: &mut Ui) {
        if !self.call_panel_shown() {
            self.room.panel_rect = None;
            return;
        }
        let palette = self.side;
        let voice = self.room.voice.clone();
        let sharing = self.room.streams.outgoing.is_some();
        let (title, waiting, place, leave_tip, buttons) = match (&voice, &self.manual) {
            (Some(voice), _) => {
                let mut buttons = vec![
                    (PanelAction::Camera, icons::CAMERA, if self.camera_on() { Look::Primary } else { Look::Plain }, true, if self.camera_on() { "Desligar câmera" } else { "Ligar câmera" }),
                    (PanelAction::Share, icons::SCREEN, if sharing { Look::Live } else { Look::Plain }, true, if sharing { "Parar de transmitir" } else { "Transmitir tela" }),
                ];
                if self.outgoing_live() {
                    buttons.push((PanelAction::Settings, icons::SLIDERS, Look::Plain, true, "Qualidade e som"));
                }
                buttons.push((PanelAction::Sounds, icons::SOUNDBOARD, Look::Plain, true, "Mesa de sons"));
                buttons.push((PanelAction::Debug, icons::PULSE, Look::Plain, true, "Depuração"));
                ("Voz conectada", false, self.place_name(voice), "Desconectar", buttons)
            }
            (None, Some(manual)) => {
                let live = manual.live();
                let share = manual.role() == ManualRole::Share;
                let title = match (live, share) {
                    (true, true) => "Transmitindo",
                    (true, false) => "Assistindo",
                    (false, _) => "Conectando…",
                };
                let mut buttons = Vec::new();
                if share {
                    buttons.push((PanelAction::Switch, icons::SCREEN, Look::Plain, live, "Trocar tela"));
                    buttons.push((PanelAction::Settings, icons::SLIDERS, Look::Plain, live, "Qualidade e som"));
                } else {
                    let full = manual.fullscreen();
                    buttons.push((PanelAction::Fullscreen, icons::EXPAND, if full { Look::Primary } else { Look::Plain }, live, if full { "Sair da tela cheia" } else { "Tela cheia" }));
                }
                buttons.push((PanelAction::Debug, icons::PULSE, Look::Plain, true, "Depuração"));
                (title, !live, "Conexão com código".to_owned(), "Encerrar", buttons)
            }
            (None, None) => return,
        };

        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), CALL_PANEL_HEIGHT), Sense::hover());
        self.room.panel_rect = Some(rect);
        // The card under it, and the line between it and the person, are drawn by the sidebar.
        let inner = Rect::from_min_max(rect.min + Vec2::splat(8.0), rect.max - Vec2::new(8.0, 9.0));
        let mut action = None;

        // The first line: the state of the connection, which opens the balloon about it, and the way out.
        let leave = Rect::from_min_size(egui::pos2(inner.right() - 32.0, inner.top()), Vec2::splat(32.0));
        let leave_response = ui.interact(leave, ui.scope_id().with("call-leave"), Sense::click());
        if leave_response.hovered() {
            ui.painter().rect_filled(leave, CornerRadius::same(4), palette.hover);
        }
        let ink = if leave_response.hovered() { palette.danger } else { palette.muted };
        icons::HANGUP.image(20.0, ink).paint_at(ui, Rect::from_center_size(leave.center(), Vec2::splat(20.0)));
        if leave_response.on_hover_text(leave_tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            action = Some(PanelAction::Leave);
        }
        let title_color = if waiting { palette.yellow } else { palette.green };
        let galley = ui.painter().layout_no_wrap(title.to_owned(), FontId::new(14.0, theme::semibold()), title_color);
        let status = Rect::from_min_max(egui::pos2(inner.left(), leave.center().y - 13.0), egui::pos2(leave.left() - 4.0, leave.center().y + 13.0));
        self.room.status_rect = Some(status);
        let status_response = ui.interact(status, ui.scope_id().with("call-status"), Sense::click());
        // The bars take the color of the last reading, as `.call-signal` did.
        let signal = match self.debug.quality("call") {
            crate::debug_view::Quality::Unknown => palette.muted,
            crate::debug_view::Quality::Good => palette.green,
            crate::debug_view::Quality::Medium => palette.yellow,
            crate::debug_view::Quality::Bad => palette.danger,
        };
        icons::SIGNAL.image(16.0, signal).paint_at(ui, Rect::from_min_size(egui::pos2(status.left() + 4.0, status.center().y - 8.0), Vec2::splat(16.0)));
        let text_left = status.left() + 4.0 + 16.0 + 6.0;
        let text_rect = Rect::from_min_size(egui::pos2(text_left, status.center().y - galley.size().y / 2.0), galley.size());
        ui.painter().with_clip_rect(status).galley(text_rect.min, galley, title_color);
        if status_response.hovered() {
            ui.painter().hline(text_rect.x_range(), text_rect.bottom() - 2.0, Stroke::new(1.0, title_color));
        }
        if status_response.on_hover_text("Ver a depuração da conexão").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            action = Some(PanelAction::Status);
        }

        // The second line: where, which leads there.
        let line_top = inner.top() + 32.0;
        let room = (inner.width() - 8.0).max(0.0);
        let mut job = egui::text::LayoutJob::simple_singleline(place, FontId::proportional(12.0), palette.muted);
        job.wrap = egui::text::TextWrapping { max_width: room, max_rows: 1, break_anywhere: true, ..Default::default() };
        let galley = ui.painter().layout_job(job);
        let open = Rect::from_min_size(egui::pos2(inner.left(), line_top), Vec2::new(galley.size().x + 8.0, 16.0));
        let open_response = ui.interact(open, ui.scope_id().with("call-open"), Sense::click());
        let place_color = if open_response.hovered() { palette.text } else { palette.muted };
        let text_pos = egui::pos2(open.left() + 4.0, open.center().y - galley.size().y / 2.0);
        let width = galley.size().x;
        ui.painter().galley_with_override_text_color(text_pos, galley, place_color);
        if open_response.hovered() {
            ui.painter().hline(text_pos.x..=text_pos.x + width, open.bottom() - 1.0, Stroke::new(1.0, place_color));
        }
        if open_response.on_hover_text("Ir para a chamada").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            action = Some(PanelAction::Open);
        }

        // The third line: the buttons, each as wide as its share.
        let row = Rect::from_min_size(egui::pos2(inner.left(), line_top + 16.0 + 8.0), Vec2::new(inner.width(), 32.0));
        let count = buttons.len() as f32;
        let width = (row.width() - 6.0 * (count - 1.0)) / count;
        for (index, (which, icon, look, enabled, tip)) in buttons.into_iter().enumerate() {
            let place = Rect::from_min_size(egui::pos2(row.left() + index as f32 * (width + 6.0), row.top()), Vec2::new(width, 32.0));
            if which == PanelAction::Sounds {
                self.room.sounds_rect = Some(place);
            }
            if panel_button(ui, &palette, place, icon, look, enabled, tip) {
                action = Some(which);
            }
        }

        let manual = self.manual.as_ref().map(|manual| (manual.fullscreen(), manual.stopping()));
        match action {
            Some(PanelAction::Leave) if voice.is_some() => self.hub.leave_voice(),
            Some(PanelAction::Leave) => {
                if manual.is_some_and(|(_, stopping)| !stopping) {
                    self.stop_manual_session();
                }
            }
            // Out of a call there is no voice to tell about, so the state opens the debug window.
            Some(PanelAction::Status) if voice.is_none() => self.open_debug(),
            Some(PanelAction::Status) => {
                self.room.popover = !self.room.popover;
                self.room.popover_privacy = false;
            }
            Some(PanelAction::Open) => match &voice {
                Some(voice) if self.contacts.iter().any(|(contact, _)| contact.id == voice.space_id) => self.navigate(Nav::Direct(voice.space_id.clone())),
                Some(voice) => self.navigate(Nav::Server { server_id: voice.space_id.clone(), channel_id: Some(voice.room_id.clone()) }),
                None => self.navigate(Nav::Home),
            },
            Some(PanelAction::Camera) => self.set_camera(!self.camera_on()),
            Some(PanelAction::Share) => self.toggle_share(),
            Some(PanelAction::Switch) => self.open_picker(true, true),
            Some(PanelAction::Settings) => self.open_stream_settings(),
            Some(PanelAction::Fullscreen) => self.set_manual_fullscreen(!manual.is_some_and(|(full, _)| full)),
            Some(PanelAction::Sounds) => self.toggle_soundboard(),
            Some(PanelAction::Debug) => self.open_debug(),
            None => {}
        }
    }

    fn toggle_share(&mut self) {
        if self.room.streams.outgoing.is_some() {
            self.hub.stop_live();
        } else {
            self.open_picker(false, false);
        }
    }

    /// The room above the chat of a voice channel or of a contact: the lobby before entering,
    /// the stage after. Returns the height it used.
    pub(crate) fn room_panel(&mut self, ui: &mut Ui, space_id: &str, room_id: &str) {
        self.drain_tasks();
        let palette = self.palette;
        let people = self.people_in(space_id, room_id);
        let joined = self.in_room(space_id, room_id);
        if !joined && room_id == DM_ROOM && people.is_empty() {
            return;
        }
        if !joined {
            self.lobby(ui, &palette, space_id, room_id, &people);
            return;
        }
        self.stage(ui, &palette, space_id, room_id, people);
    }

    /// The card of a room the person is not in: what it is, who is there and the way in.
    fn lobby(&mut self, ui: &mut Ui, palette: &Palette, space_id: &str, room_id: &str, people: &[Person]) {
        let contact = self.contacts.iter().find(|(contact, _)| contact.id == space_id).map(|(contact, _)| contact.name.clone());
        let live = people.iter().any(|person| person.live);
        let (title, text, join) = match contact {
            Some(name) => (
                if live { format!("{name} está transmitindo") } else { format!("{name} está em uma chamada") },
                "Entre para conversar por voz e ver a tela ou a câmera.".to_owned(),
                "Entrar na chamada",
            ),
            None => {
                let room = self.servers.iter().find(|server| server.id == space_id).and_then(|server| server.channels.iter().find(|channel| channel.id == room_id));
                let text = match people.len() {
                    0 => "Ninguém na sala. Entre e chame o pessoal.".to_owned(),
                    1 => "1 pessoa na sala agora.".to_owned(),
                    count => format!("{count} pessoas na sala agora."),
                };
                (room.map_or_else(|| "Sala".to_owned(), |room| room.name.clone()), text, "Entrar na sala")
            }
        };
        let mut enter = false;
        let glass = palette.glass.then(|| crate::glass::reserve(ui.painter()));
        let card = if palette.glass {
            Frame::new().outer_margin(Margin { left: 16, right: 16, top: 16, bottom: 0 }).inner_margin(Margin::symmetric(20, 16)).corner_radius(CornerRadius::same(14))
        } else {
            Frame::new()
                .outer_margin(Margin { left: 16, right: 16, top: 16, bottom: 0 })
                .inner_margin(Margin::symmetric(20, 16))
                .corner_radius(CornerRadius::same(8))
                .fill(palette.sidebar)
                .stroke(Stroke::new(1.0, palette.line_soft))
        };
        let shown = card
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    let (art, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
                    ui.painter().circle_filled(art.center(), 28.0, palette.main);
                    icons::SPEAKER.image(30.0, palette.green).paint_at(ui, Rect::from_center_size(art.center(), Vec2::splat(30.0)));
                    ui.vertical(|ui| {
                        ui.set_width((ui.available_width() - 170.0).max(120.0));
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new(title).color(palette.strong).size(18.0).family(theme::bold()));
                        ui.label(RichText::new(text).color(palette.muted).size(15.0));
                        if !people.is_empty() {
                            ui.add_space(6.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                                for person in people {
                                    let name = ui.painter().layout_no_wrap(person.name.clone(), FontId::proportional(13.0), palette.text);
                                    let badge = if person.live { 56.0 } else { 0.0 };
                                    let (chip, _) = ui.allocate_exact_size(Vec2::new(2.0 + 24.0 + 6.0 + name.size().x + badge + 8.0, 28.0), Sense::hover());
                                    ui.painter().rect_filled(chip, CornerRadius::same(14), palette.raised);
                                    let face = Rect::from_min_size(chip.min + Vec2::splat(2.0), Vec2::splat(24.0));
                                    let picture = self.pictures.avatar(&self.hub, person.member_id.as_deref());
                                    widgets::paint_avatar(ui, face, &person.name, picture);
                                    let left = face.right() + 6.0;
                                    ui.painter().galley(egui::pos2(left, chip.center().y - name.size().y / 2.0), name.clone(), palette.text);
                                    if person.live {
                                        let mark = Rect::from_min_size(egui::pos2(left + name.size().x + 6.0, chip.center().y - 8.0), Vec2::new(48.0, 16.0));
                                        ui.painter().rect_filled(mark, CornerRadius::same(4), LIVE_RED);
                                        ui.painter().text(mark.center(), Align2::CENTER_CENTER, "AO VIVO", FontId::new(10.0, theme::bold()), Color32::WHITE);
                                    }
                                }
                            });
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        enter = button(ui, palette, join, Tone::Primary).clicked();
                    });
                });
            });
        if let Some(reserved) = glass {
            // A `.card` of the glass theme.
            let rect = shown.response.rect;
            let pane = crate::glass::Pane { blur: Some(12.0), shine: false, ..crate::glass::Pane::new(Color32::from_white_alpha(15), 14.0) };
            crate::glass::fill(ui.painter(), reserved, rect, &pane);
        }
        if enter {
            self.join_room(space_id, room_id);
        }
    }

    /// The stage of the call, as `.room-stage` drew it: the tiles, the line of feedback, the
    /// round buttons and the handle that sets the height of the tiles.
    fn stage(&mut self, ui: &mut Ui, palette: &Palette, space_id: &str, room_id: &str, people: Vec<Person>) {
        let window = ui.ctx().content_rect().height();
        let cap = stage_cap(window, room_id != DM_ROOM, self.prefs.stage_height);
        let mut tiles: Vec<Tile> = self
            .room
            .streams
            .incoming
            .iter()
            .filter(|entry| entry.space_id == space_id && entry.room_id == room_id)
            .filter(|entry| !people.iter().any(|person| person.id == entry.sharer_id))
            .map(|entry| Tile::Stream { sharer_id: entry.sharer_id.clone(), name: entry.name.clone(), open: entry.phase == IncomingPhase::Live, embedded: entry.embedded })
            .collect();
        tiles.extend(people.into_iter().map(Tile::Person));
        let focus = self.room.focus.as_ref().and_then(|id| tiles.iter().position(|tile| &tile.id() == id));
        let full = ui.available_width();
        let inner_width = (full - 32.0).max(0.0);
        let (places, box_height) = place_tiles(tiles.len(), focus, inner_width, cap, self.prefs.stage_height.is_some());

        let feedback = (!self.room.feedback.is_empty())
            .then(|| ui.painter().layout(self.room.feedback.clone(), FontId::proportional(13.0), palette.text, inner_width.max(10.0)));
        let feedback_height = feedback.as_ref().map_or(0.0, |galley| TILE_GAP + galley.size().y);
        let height = 16.0 + box_height + feedback_height + TILE_GAP + 48.0 + 6.0 + 10.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(full, height + 1.0), Sense::hover());
        // On glass the stage darkens the panel instead of covering it: `.stage { background: rgba(0, 0, 0, .35) }`.
        let stage = if palette.glass { Color32::from_black_alpha(89) } else { STAGE };
        ui.painter().rect_filled(Rect::from_min_max(rect.min, rect.max - Vec2::new(0.0, 1.0)), CornerRadius::ZERO, stage);
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, palette.line));

        let tiles_box = Rect::from_min_size(rect.min + Vec2::splat(16.0), Vec2::new(inner_width, box_height));
        let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(tiles_box));
        inside.set_clip_rect(tiles_box.intersect(ui.clip_rect()));
        for (index, place, small) in places {
            self.draw_tile(&mut inside, palette, &tiles, index, place.translate(tiles_box.min.to_vec2()), small);
        }

        let mut top = tiles_box.bottom() + TILE_GAP;
        if let Some(galley) = feedback {
            let size = galley.size();
            ui.painter().galley(egui::pos2(rect.center().x - size.x / 2.0, top), galley, palette.text);
            top += size.y + TILE_GAP;
        }
        self.stage_buttons(ui, palette, rect.center().x, top);
        let handle = Rect::from_min_max(egui::pos2(rect.left(), top + 48.0 + 6.0), egui::pos2(rect.right(), rect.bottom() - 1.0));
        self.stage_handle(ui, palette, handle, window, box_height);
    }

    fn outgoing_live(&self) -> bool {
        self.room.streams.outgoing.as_ref().is_some_and(|outgoing| outgoing.phase == OutgoingPhase::Live)
    }

    /// The round buttons under the tiles.
    fn stage_buttons(&mut self, ui: &mut Ui, palette: &Palette, center: f32, top: f32) {
        let Some(voice) = self.room.voice.clone() else { return };
        let mic_live = voice.mic && !voice.deaf;
        let sharing = self.room.streams.outgoing.is_some();
        let mut buttons = vec![
            (Round::Mic, if mic_live { icons::MIC } else { icons::MIC_OFF }, if mic_live { Look::Plain } else { Look::Off }, if mic_live { "Silenciar microfone" } else { "Ativar microfone" }),
            (Round::Camera, icons::CAMERA, if self.camera_on() { Look::On } else { Look::Plain }, if self.camera_on() { "Desligar câmera" } else { "Ligar câmera" }),
            (Round::Share, icons::SCREEN, if sharing { Look::Live } else { Look::Plain }, if sharing { "Parar de transmitir" } else { "Transmitir tela" }),
        ];
        if self.outgoing_live() {
            buttons.push((Round::Switch, icons::SWITCH_SCREEN, Look::Plain, "Trocar tela"));
            buttons.push((Round::Settings, icons::SLIDERS, Look::Plain, "Qualidade e som da transmissão"));
        }
        buttons.push((Round::Leave, icons::HANGUP, Look::Danger, "Desconectar"));
        let count = buttons.len() as f32;
        let mut x = center - (count * 48.0 + (count - 1.0) * 12.0) / 2.0;
        let mut clicked = None;
        for (which, icon, look, tip) in buttons {
            let rect = Rect::from_min_size(egui::pos2(x, top), Vec2::splat(48.0));
            x += 60.0;
            let id = ui.scope_id().with(("stage-button", format!("{which:?}")));
            let response = ui.interact(rect, id, Sense::click());
            let hovered = response.hovered();
            // The hover and the change of state ease in and out instead of jumping, and the
            // button grows a little under the pointer.
            let ctx = ui.ctx().clone();
            let lit = crate::motion::hover(&ctx, id, hovered);
            let grow = crate::motion::lerp(1.0, 1.07, lit);
            let radius = 24.0 * grow;
            let circle = Rect::from_center_size(rect.center(), Vec2::splat(radius * 2.0));
            let (base, over, ink) = match look {
                Look::Plain => (palette.raised, palette.selected, palette.strong),
                Look::Off => (palette.strong, palette.selected, palette.danger),
                // As `.room-control.on`: the light circle with a dark icon.
                Look::On => (palette.strong, palette.text, Color32::from_rgb(0x11, 0x11, 0x11)),
                Look::Live => (palette.green, palette.selected, Color32::WHITE),
                Look::Danger | Look::Primary => (palette.danger, palette.danger_hover, Color32::WHITE),
            };
            let base = crate::motion::color(&ctx, id.with("base"), base, crate::motion::STATE);
            let fill = crate::motion::mix(base, over, lit);
            let mut ink = crate::motion::color(&ctx, id.with("ink"), ink, crate::motion::STATE);
            if palette.glass && matches!(look, Look::Plain | Look::Off | Look::Danger) {
                // `.round` on glass, the muted mic too: white at 12% over a blur, with the light
                // edge, and the icon keeps its color. The hang up looks like the others until the
                // pointer is on it, and then it glows red.
                let danger = look == Look::Danger;
                if danger {
                    ink = crate::motion::mix(palette.strong, Color32::WHITE, lit);
                    let glow = egui::Shadow { offset: [0, 6], blur: 22, spread: 0, color: palette.danger.gamma_multiply(0.55 * lit) };
                    ui.painter().add(glow.as_shape(circle, CornerRadius::same(radius.round() as u8)));
                }
                let tint = Color32::from_white_alpha(crate::motion::lerp(31.0, 46.0, if danger { 0.0 } else { lit }).round() as u8);
                crate::glass::pane(ui.painter(), circle, &crate::glass::Pane { blur: Some(12.0), shine: false, ..crate::glass::Pane::new(tint, radius) });
                if danger && lit > 0.0 {
                    ui.painter().circle_filled(circle.center(), radius, palette.danger.gamma_multiply(lit));
                    ui.painter().circle_stroke(circle.center(), radius - 0.5, Stroke::new(1.0, Color32::from_white_alpha((46.0 * lit) as u8)));
                }
            } else {
                ui.painter().circle_filled(circle.center(), radius, fill);
            }
            let size = 22.0 * grow;
            icon.image(size, ink).paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(size)));
            if response.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(which);
            }
        }
        match clicked {
            Some(Round::Mic) => {
                let on = !voice.mic || voice.deaf;
                self.hub.update_voice(Some(on), None, (on && voice.deaf).then_some(false));
            }
            Some(Round::Camera) => self.set_camera(!self.camera_on()),
            Some(Round::Share) => self.toggle_share(),
            Some(Round::Switch) => self.open_picker(true, false),
            Some(Round::Settings) => self.open_stream_settings(),
            Some(Round::Leave) => self.hub.leave_voice(),
            None => {}
        }
    }

    /// The strip at the foot of the stage that is dragged to make the tiles taller or shorter.
    fn stage_handle(&mut self, ui: &mut Ui, palette: &Palette, rect: Rect, window: f32, box_height: f32) {
        let response = ui
            .interact(rect, ui.scope_id().with("stage-handle"), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text("Arraste para mudar o tamanho. Dois cliques voltam ao padrão.");
        let lit = response.hovered() || response.dragged();
        let grip = Rect::from_min_size(egui::pos2(rect.center().x - 24.0, rect.top() + 3.0), Vec2::new(48.0, 4.0));
        ui.painter().rect_filled(grip, CornerRadius::same(2), if lit { palette.muted } else { palette.line_soft });
        let pointer = ui.input(|input| input.pointer.interact_pos()).map(|pos| pos.y);
        if response.double_clicked() {
            self.room.resizing = None;
            self.prefs.stage_height = None;
            self.prefs.save();
            return;
        }
        if response.drag_started() {
            self.room.resizing = pointer.map(|y| (y, box_height));
        }
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            if let (Some((from, start)), Some(y)) = (self.room.resizing, pointer) {
                let limit = (window - STAGE_RESERVED).max(140.0);
                self.prefs.stage_height = Some((start + y - from).clamp(140.0, limit).round());
            }
        }
        if response.drag_stopped() {
            self.room.resizing = None;
            self.prefs.save();
        }
    }

    /// One tile of the stage, with what a click on it does.
    fn draw_tile(&mut self, ui: &mut Ui, palette: &Palette, tiles: &[Tile], index: usize, rect: Rect, small: bool) {
        let tile = &tiles[index];
        let id = tile.id();
        let response = ui.interact(rect, ui.scope_id().with(("tile", &id)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let person = match tile {
            Tile::Person(person) => Some(person),
            Tile::Stream { sharer_id, .. } => tiles.iter().find_map(|tile| match tile {
                Tile::Person(person) if &person.id == sharer_id => Some(person),
                _ => None,
            }),
        };
        if let Some(person) = person {
            let space_id = self.room.voice.as_ref().map(|voice| voice.space_id.clone()).unwrap_or_default();
            self.person_menu(&response, person, &space_id, true);
        }
        if response.clicked() {
            self.room.focus = if self.room.focus.as_deref() == Some(id.as_str()) { None } else { Some(id) };
        }
        let hovered = ui.rect_contains_pointer(rect);
        if palette.glass {
            // `.tile` on glass: white at 7% with a light edge, rounder.
            ui.painter().rect_filled(rect, CornerRadius::same(14), Color32::from_white_alpha(18));
            ui.painter().rect_stroke(rect, CornerRadius::same(14), Stroke::new(1.0, palette.line_soft), egui::StrokeKind::Inside);
        } else {
            ui.painter().rect_filled(rect, CornerRadius::same(8), palette.sidebar);
        }
        match tile {
            Tile::Person(person) => self.person_tile(ui, palette, person, rect, small),
            Tile::Stream { sharer_id, name, open, embedded } => {
                let store = if *embedded { self.stream_store(sharer_id) } else { None };
                let texture = store.and_then(|store| self.video_texture(ui.ctx(), &format!("stream:{sharer_id}"), &store, rect.size()));
                let connected = *embedded && *open && texture.is_some();
                match texture.filter(|_| connected) {
                    Some(texture) => paint_video(ui, rect, texture, false, false),
                    None => self.stream_note(ui, palette, rect, sharer_id, name, !*embedded),
                }
                tile_label(ui, rect, &format!("Tela de {name}"), None, 1.0);
                live_badge(ui, rect, palette);
                if hovered {
                    self.stream_tools(ui, rect, sharer_id, name, connected, *embedded, true);
                }
            }
        }
    }

    /// As `.stream-note`: what is happening with a screen not shown yet, and the way back from the
    /// window of the engine.
    fn stream_note(&mut self, ui: &mut Ui, palette: &Palette, rect: Rect, sharer_id: &str, name: &str, native: bool) {
        let note = if native { format!("A tela de {name} está aberta em outra janela") } else { format!("Conectando à tela de {name}…") };
        let job = egui::text::LayoutJob {
            halign: Align::Center,
            ..egui::text::LayoutJob::simple(note, FontId::proportional(13.0), palette.muted, (rect.width() - 32.0).max(10.0))
        };
        let galley = ui.painter().layout_job(job);
        let button_height = if native { 10.0 + 28.0 } else { 0.0 };
        let top = rect.center().y - (galley.size().y + button_height) / 2.0;
        let text_height = galley.size().y;
        ui.painter().galley(egui::pos2(rect.center().x, top), galley, palette.muted);
        if native {
            let label = ui.painter().layout_no_wrap("Trazer para o app".to_owned(), FontId::new(13.0, theme::semibold()), palette.strong);
            let back = Rect::from_center_size(egui::pos2(rect.center().x, top + text_height + 10.0 + 14.0), Vec2::new(label.size().x + 20.0, 28.0));
            let response = ui.interact(back, ui.scope_id().with(("stream-popin", sharer_id)), Sense::click());
            ui.painter().rect_filled(back, CornerRadius::same(4), if response.hovered() { palette.muted.gamma_multiply(0.55) } else { palette.selected });
            ui.painter().galley(back.center() - label.size() / 2.0, label, palette.strong);
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.hub.set_stream_window(sharer_id, false);
            }
        }
    }

    /// As `.stream-tools`: the buttons at the top right of a screen, while the pointer is over it.
    #[allow(clippy::too_many_arguments)]
    fn stream_tools(&mut self, ui: &mut Ui, rect: Rect, sharer_id: &str, name: &str, connected: bool, embedded: bool, close: bool) {
        let mut tools = Vec::new();
        if connected {
            tools.push((icons::EXPAND, "Tela cheia"));
        }
        if embedded {
            tools.push((icons::POPOUT, "Abrir em outra janela"));
        }
        if close {
            tools.push((icons::CLOSE, "Parar de assistir"));
        }
        let mut right = rect.right() - 8.0;
        let mut picked = None;
        for (icon, tip) in tools.into_iter().rev() {
            let place = Rect::from_min_size(egui::pos2(right - 30.0, rect.top() + 8.0), Vec2::splat(30.0));
            right -= 30.0 + 6.0;
            let button = ui.interact(place, ui.scope_id().with(("stream-tool", sharer_id, tip)), Sense::click());
            let shade = if button.hovered() { Color32::from_black_alpha(230) } else { Color32::from_black_alpha(166) };
            ui.painter().rect_filled(place, CornerRadius::same(4), shade);
            icon.image(18.0, Color32::WHITE).paint_at(ui, Rect::from_center_size(place.center(), Vec2::splat(18.0)));
            if button.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                picked = Some(tip);
            }
        }
        match picked {
            Some("Tela cheia") => self.open_viewer(sharer_id, format!("Tela de {name}")),
            Some("Abrir em outra janela") => self.hub.set_stream_window(sharer_id, true),
            Some(_) => self.hub.unwatch(sharer_id),
            None => {}
        }
    }

    fn person_tile(&mut self, ui: &mut Ui, palette: &Palette, person: &Person, rect: Rect, small: bool) {
        // The camera of this app shows as a mirror, as `.mirror` did; someone else's as it comes,
        // unless it was turned off here.
        let key = person.member_id.clone().unwrap_or_else(|| person.id.clone());
        // The screen of this person, when it is watched here: inside the window, or in the
        // window of the engine.
        let watched = if person.me {
            None
        } else {
            self.room.streams.incoming.iter().find(|entry| entry.sharer_id == person.id && entry.phase == IncomingPhase::Live).map(|entry| entry.embedded)
        };
        let screen = if watched == Some(true) {
            self.stream_store(&person.id).and_then(|store| self.video_texture(ui.ctx(), &format!("stream:{}", person.id), &store, rect.size()))
        } else {
            None
        };
        if let Some(texture) = screen {
            paint_video(ui, rect, texture, false, false);
            if ui.rect_contains_pointer(rect) {
                self.stream_tools(ui, rect, &person.id, &person.name, true, true, false);
            }
        } else if watched == Some(false) {
            self.stream_note(ui, palette, rect, &person.id, &person.name, true);
        } else {
            self.person_picture(ui, person, rect, small, &key);
        }
        self.person_overlay(ui, palette, person, rect, small);
    }

    /// The face of a tile: the camera, the own screen going out, the small picture of a screen,
    /// or the avatar.
    fn person_picture(&mut self, ui: &mut Ui, person: &Person, rect: Rect, small: bool, key: &str) {
        // Without the camera, the tile of the person sharing shows their own screen, as it goes out.
        let own_screen = person.me && !self.camera_on() && person.live;
        let video = if own_screen {
            self.stream_store(SELF_VIEWER).map(|store| (format!("stream:{SELF_VIEWER}"), store))
        } else if person.me {
            self.camera_on().then(|| self.room.camera_feed.as_ref().map(|feed| Arc::clone(feed.preview()))).flatten().map(|store| (SELF_VIDEO.to_owned(), store))
        } else if person.camera && !self.prefs.video_hidden.iter().any(|hidden| hidden.as_str() == key) {
            self.room.call.as_ref().and_then(|call| call.call.video_of(&person.id)).map(|store| (person.id.clone(), store))
        } else {
            None
        };
        let texture = video.and_then(|(id, store)| self.video_texture(ui.ctx(), &id, &store, rect.size()));
        let thumb = if person.live && !person.me && texture.is_none() { self.hub.stream_thumb(&person.id) } else { None };
        if let Some(texture) = texture {
            paint_video(ui, rect, texture, !own_screen, person.me && !own_screen);
            // As `.tile-expand`: the video over the whole screen, while the pointer is over the tile.
            if ui.rect_contains_pointer(rect) {
                let place = Rect::from_min_size(egui::pos2(rect.right() - 38.0, rect.top() + 8.0), Vec2::splat(30.0));
                let button = ui.interact(place, ui.scope_id().with(("tile-expand", &person.id)), Sense::click());
                let shade = if button.hovered() { Color32::from_black_alpha(230) } else { Color32::from_black_alpha(153) };
                ui.painter().rect_filled(place, CornerRadius::same(4), shade);
                icons::EXPAND.image(18.0, Color32::WHITE).paint_at(ui, Rect::from_center_size(place.center(), Vec2::splat(18.0)));
                if button.on_hover_text("Tela cheia").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    let (source, label) = if own_screen {
                        (ViewSource::Stream(SELF_VIEWER.to_owned()), "Sua tela".to_owned())
                    } else if person.me {
                        (ViewSource::OwnCamera, format!("{} (você)", person.name))
                    } else {
                        (ViewSource::Camera(person.id.clone()), person.name.clone())
                    };
                    self.open_tile_viewer(source, label);
                }
            }
        } else if let Some(thumb) = thumb {
            ui.painter().rect_filled(rect, CornerRadius::same(8), Color32::BLACK);
            let uri = format!("bytes://thumb/{}/{}.jpg", person.id, thumb.len());
            let image = egui::Image::from_bytes(uri, egui::load::Bytes::Shared(thumb));
            let size = image.load_for_size(ui.ctx(), rect.size()).ok().and_then(|poll| poll.size()).unwrap_or(Vec2::new(16.0, 9.0));
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            image.paint_at(ui, Rect::from_center_size(rect.center(), size * scale));
        } else {
            let (side, letter) = if small { (40.0, 18.0) } else { (72.0, 30.0) };
            let picture = self.pictures.avatar(&self.hub, person.member_id.as_deref());
            widgets::paint_avatar_with(ui, Rect::from_center_size(rect.center(), Vec2::splat(side)), &person.name, picture, letter);
        }
    }

    /// What goes over the face of a tile: the speaking ring, the name, the state of the
    /// connection, and the buttons of a share.
    fn person_overlay(&mut self, ui: &mut Ui, palette: &Palette, person: &Person, rect: Rect, small: bool) {
        if self.is_speaking(&person.id) {
            ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(3.0, palette.green), egui::StrokeKind::Inside);
        }
        let label = if person.me { format!("{} (você)", person.name) } else { person.name.clone() };
        let muted = (person.deaf || !person.mic).then_some((if person.deaf { icons::DEAF } else { icons::MIC_OFF }, palette.danger_text));
        tile_label(ui, rect, &label, muted, 1.0);
        if !person.me {
            if let Some(note) = self.peer_note(&person.id) {
                let galley = ui.painter().layout_no_wrap(note.to_owned(), FontId::proportional(12.0), palette.yellow);
                let size = galley.size() + Vec2::new(16.0, 4.0);
                let status = Rect::from_min_size(egui::pos2(rect.center().x - size.x / 2.0, rect.top() + 8.0), size);
                ui.painter().rect_filled(status, CornerRadius::same(4), SHADE);
                ui.painter().galley(status.min + Vec2::new(8.0, 2.0), galley, palette.yellow);
            }
        }
        if !person.live {
            return;
        }
        live_badge(ui, rect, palette);

        let scale = if small { 0.85 } else { 1.0 };
        let salt = format!("actions-{}", person.id);
        if person.me {
            let watching = self.room.streams.outgoing.as_ref().map_or(0, |outgoing| outgoing.viewers.iter().filter(|(_, state)| state == "Connected").count());
            let note = if watching == 1 { "1 assistindo".to_owned() } else { format!("{watching} assistindo") };
            match tile_actions(ui, palette, rect, &[Piece::Note(note), Piece::Button("Trocar", Look::Plain), Piece::Button("Parar", Look::Danger)], scale, &salt) {
                Some(1) => self.open_picker(true, false),
                Some(2) => self.hub.stop_live(),
                _ => {}
            }
            return;
        }
        let watching = self.room.streams.incoming.iter().find(|entry| entry.sharer_id == person.id).map(|entry| entry.phase);
        let pieces = match watching {
            None => vec![Piece::Button("Assistir", Look::Primary)],
            Some(IncomingPhase::Live) => vec![Piece::Note("Assistindo".to_owned()), Piece::Button("Parar de assistir", Look::Plain)],
            Some(_) => vec![Piece::Note("Abrindo…".to_owned()), Piece::Button("Cancelar", Look::Plain)],
        };
        if tile_actions(ui, palette, rect, &pieces, scale, &salt).is_none() {
            return;
        }
        if watching.is_none() {
            if let Err(error) = self.hub.watch(&person.id) {
                self.notice(format!("Não consegui assistir: {error}."));
            }
        } else {
            self.hub.unwatch(&person.id);
        }
    }

}

const TILE_GAP: f32 = 10.0;
const TILE_MIN_WIDTH: f32 = 214.0;
/// What the stage leaves of the window for the chat under it.
const STAGE_RESERVED: f32 = 340.0;
/// The panel of the call over the strip of the person: padding, two lines, the buttons, the line.
pub(crate) const CALL_PANEL_HEIGHT: f32 = 105.0;
const SHADE: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 153);
const ACTION: Color32 = Color32::from_rgb(0x4e, 0x50, 0x58);
const ACTION_HOVER: Color32 = Color32::from_rgb(0x6d, 0x6f, 0x78);
/// Wakes the window for a new frame of video. A repaint asked with no delay makes egui draw the
/// window twice, which would double the work of every frame; any delay above zero draws it once.
pub(crate) fn video_waker(context: egui::Context) -> telinha_voice::video::Waker {
    Arc::new(move || context.request_repaint_after(std::time::Duration::from_millis(1)))
}

/// A frame as the small JPEG picture that goes to the room, 320 wide, as `captureThumb`.
fn thumb_of(picture: &telinha_voice::video::Picture) -> Option<String> {
    let full = image::RgbaImage::from_raw(picture.width, picture.height, picture.rgba.clone())?;
    let scale = (320.0 / picture.width as f32).min(1.0);
    let (width, height) = (((picture.width as f32 * scale).round() as u32).max(2), ((picture.height as f32 * scale).round() as u32).max(2));
    let small = image::imageops::resize(&full, width, height, image::imageops::FilterType::Triangle);
    let rgb = image::DynamicImage::ImageRgba8(small).to_rgb8();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 55).encode_image(&rgb).ok()?;
    Some(format!("data:image/jpeg;base64,{}", telinha_core::text::encode_base64(&bytes)))
}

/// The key of the picture of the camera of this app among the textures.
const SELF_VIDEO: &str = "self:camera";

/// Draws a frame of video over a tile, filling it and cut at the sides when `cover`, whole
/// inside it otherwise, and turned as a mirror when asked.
pub(crate) fn paint_video(ui: &Ui, rect: Rect, texture: egui::load::SizedTexture, cover: bool, mirror: bool) {
    let size = texture.size;
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let (place, mut uv) = if cover {
        let scale = (rect.width() / size.x).max(rect.height() / size.y);
        let seen = Vec2::new(rect.width() / (size.x * scale), rect.height() / (size.y * scale));
        (rect, Rect::from_center_size(egui::pos2(0.5, 0.5), seen))
    } else {
        let scale = (rect.width() / size.x).min(rect.height() / size.y);
        ui.painter().rect_filled(rect, CornerRadius::same(8), Color32::BLACK);
        (Rect::from_center_size(rect.center(), size * scale), Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)))
    };
    if mirror {
        std::mem::swap(&mut uv.min.x, &mut uv.max.x);
    }
    let radius = if cover { CornerRadius::same(8) } else { CornerRadius::ZERO };
    ui.painter().add(egui::epaint::RectShape::filled(place, radius, Color32::WHITE).with_texture(texture.id, uv));
}

/// What a tile of the stage shows: a person, or the screen of someone being watched.
enum Tile {
    Person(Person),
    Stream { sharer_id: String, name: String, open: bool, embedded: bool },
}

impl Tile {
    fn id(&self) -> String {
        match self {
            Tile::Person(person) => person.id.clone(),
            Tile::Stream { sharer_id, .. } => format!("stream:{sharer_id}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Round {
    Mic,
    Camera,
    Share,
    Switch,
    Settings,
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Look {
    Plain,
    /// A round button turned on, as the camera while it films.
    On,
    Primary,
    Danger,
    Off,
    Live,
}

enum Piece {
    Note(String),
    Button(&'static str, Look),
}

/// The width of the largest 16:9 tiles that fit `count` of them in the box, as `fitTiles` found it.
fn tile_width(count: usize, width: f32, height: f32) -> f32 {
    let mut best: f32 = 0.0;
    for columns in 1..=count.max(1) {
        let rows = count.max(1).div_ceil(columns) as f32;
        let columns = columns as f32;
        let by_width = (width - TILE_GAP * (columns - 1.0)) / columns;
        let by_height = (height - TILE_GAP * (rows - 1.0)) / rows * 16.0 / 9.0;
        best = best.max(by_width.min(by_height));
    }
    best.max(TILE_MIN_WIDTH).floor()
}

/// The highest the tiles go: what the person dragged, or a share of the window, never so tall
/// that the box to write in leaves the window.
fn stage_cap(window: f32, voice_channel: bool, dragged: Option<f32>) -> f32 {
    let limit = window - STAGE_RESERVED;
    let cap = match dragged {
        Some(height) => height.min(limit),
        None => (window * if voice_channel { 0.58 } else { 0.46 }).min(limit),
    };
    cap.max(0.0)
}

/// Where each tile goes in the box of the tiles, relative to its top left, whether it is one of
/// the small ones under a focused tile, and the height of the box.
fn place_tiles(count: usize, focus: Option<usize>, width: f32, cap: f32, fixed: bool) -> (Vec<(usize, Rect, bool)>, f32) {
    let mut places = Vec::with_capacity(count);
    if let Some(focused) = focus {
        let strip = if count > 1 { 96.0 + TILE_GAP } else { 0.0 };
        places.push((focused, Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, (cap - strip).max(0.0))), false));
        let mut x = 0.0;
        for index in (0..count).filter(|index| *index != focused) {
            places.push((index, Rect::from_min_size(egui::pos2(x, cap - 96.0), Vec2::new(170.0, 96.0)), true));
            x += 170.0 + TILE_GAP;
        }
        return (places, cap);
    }
    if count == 0 {
        return (places, if fixed { cap } else { 0.0 });
    }
    let tile = tile_width(count, width, cap).min(width);
    let height = (tile * 9.0 / 16.0).min(cap).max(120.0);
    let per_row = (((width + TILE_GAP) / (tile + TILE_GAP)).floor() as usize).max(1);
    let rows = count.div_ceil(per_row);
    let content = rows as f32 * height + (rows - 1) as f32 * TILE_GAP;
    let box_height = if fixed { cap } else { content.min(cap) };
    let top = ((box_height - content) / 2.0).max(0.0);
    for index in 0..count {
        let (row, column) = (index / per_row, index % per_row);
        let in_row = per_row.min(count - row * per_row) as f32;
        let row_width = in_row * tile + (in_row - 1.0) * TILE_GAP;
        let left = (width - row_width) / 2.0 + column as f32 * (tile + TILE_GAP);
        places.push((index, Rect::from_min_size(egui::pos2(left, top + row as f32 * (height + TILE_GAP)), Vec2::new(tile, height)), false));
    }
    (places, box_height)
}

/// The name on the bottom left corner of a tile, after the icon of a muted microphone.
fn tile_label(ui: &Ui, rect: Rect, text: &str, icon: Option<(icons::Icon, Color32)>, scale: f32) {
    let icon_width = if icon.is_some() { 16.0 + 4.0 } else { 0.0 };
    let room = (rect.width() - 16.0 - 16.0 - icon_width).max(0.0);
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), FontId::proportional(13.0 * scale), Color32::WHITE);
    job.wrap = egui::text::TextWrapping { max_width: room, max_rows: 1, break_anywhere: true, ..Default::default() };
    let galley = ui.painter().layout_job(job);
    let height = 23.0 * scale;
    let label = Rect::from_min_size(egui::pos2(rect.left() + 8.0, rect.bottom() - 8.0 - height), Vec2::new(16.0 + icon_width + galley.size().x, height));
    ui.painter().rect_filled(label, CornerRadius::same(4), SHADE);
    let mut left = label.left() + 8.0;
    if let Some((icon, color)) = icon {
        icon.image(16.0, color).paint_at(ui, Rect::from_min_size(egui::pos2(left, label.center().y - 8.0), Vec2::splat(16.0)));
        left += icon_width;
    }
    ui.painter().galley(egui::pos2(left, label.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
}

/// The red AO VIVO on the top left corner of a tile.
fn live_badge(ui: &Ui, rect: Rect, palette: &Palette) {
    let galley = ui.painter().layout_no_wrap("AO VIVO".to_owned(), FontId::new(11.0, theme::bold()), Color32::WHITE);
    let size = Vec2::new(galley.size().x + 12.0, galley.size().y.max(15.0) + 2.0);
    let badge = Rect::from_min_size(rect.left_top() + Vec2::splat(8.0), size);
    ui.painter().rect_filled(badge, CornerRadius::same(4), palette.danger);
    ui.painter().galley(badge.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

/// The notes and buttons of a tile, laid from its bottom right corner as `.tile-actions` did.
/// Returns which piece was clicked.
fn tile_actions(ui: &Ui, palette: &Palette, rect: Rect, pieces: &[Piece], scale: f32, salt: &str) -> Option<usize> {
    let row = 28.0 * scale;
    let middle = rect.bottom() - 8.0 - row / 2.0;
    let mut right = rect.right() - 8.0;
    let mut clicked = None;
    for (index, piece) in pieces.iter().enumerate().rev() {
        match piece {
            Piece::Note(text) => {
                let galley = ui.painter().layout_no_wrap(text.clone(), FontId::proportional(12.0 * scale), Color32::WHITE);
                let size = galley.size() + Vec2::new(16.0, 6.0) * scale;
                let note = Rect::from_min_size(egui::pos2(right - size.x, middle - size.y / 2.0), size);
                ui.painter().rect_filled(note, CornerRadius::same(4), SHADE);
                ui.painter().galley(note.center() - galley.size() / 2.0, galley, Color32::WHITE);
                right = note.left() - 6.0 * scale;
            }
            Piece::Button(label, look) => {
                let galley = ui.painter().layout_no_wrap((*label).to_owned(), FontId::proportional(13.0 * scale), Color32::WHITE);
                let button = Rect::from_min_size(egui::pos2(right - galley.size().x - 20.0 * scale, middle - row / 2.0), Vec2::new(galley.size().x + 20.0 * scale, row));
                let response = ui.interact(button, ui.scope_id().with((salt, index)), Sense::click());
                let lit = crate::motion::hover(ui.ctx(), response.id, response.hovered());
                let fill = match look {
                    Look::Primary => crate::motion::mix(palette.accent, palette.accent_hover, lit),
                    Look::Danger => crate::motion::mix(palette.danger, palette.danger_hover, lit),
                    _ => crate::motion::mix(ACTION, ACTION_HOVER, lit),
                };
                if palette.glass && *look == Look::Primary {
                    // `.primary` on glass: the accent gradient, rounder.
                    crate::glass::accent_fill(ui.painter(), egui::epaint::RectShape::filled(button, CornerRadius::same(8), Color32::WHITE).into());
                    if lit > 0.0 {
                        ui.painter().rect_filled(button, CornerRadius::same(8), Color32::from_white_alpha((18.0 * lit) as u8));
                    }
                } else {
                    ui.painter().rect_filled(button, CornerRadius::same(4), fill);
                }
                ui.painter().galley(button.center() - galley.size() / 2.0, galley, Color32::WHITE);
                if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    clicked = Some(index);
                }
                right = button.left() - 6.0 * scale;
            }
        }
    }
    clicked
}

/// A button of the row at the foot of the call panel, as wide as its share of the row.
fn panel_button(ui: &Ui, palette: &Palette, rect: Rect, icon: icons::Icon, look: Look, enabled: bool, tip: &str) -> bool {
    let response = ui.interact(rect, ui.scope_id().with(("call-action", tip)), if enabled { Sense::click() } else { Sense::hover() });
    let hovered = enabled && response.hovered();
    let lit = crate::motion::hover(ui.ctx(), response.id, hovered);
    let (fill, ink) = match look {
        Look::Primary => (palette.accent, Color32::WHITE),
        Look::Live => (palette.green, Color32::WHITE),
        _ => (crate::motion::mix(palette.raised, palette.selected, lit), palette.text),
    };
    let fill = crate::motion::color(ui.ctx(), response.id.with("fill"), fill, crate::motion::STATE);
    let fade = if enabled { 1.0 } else { 0.4 };
    ui.painter().rect_filled(rect, CornerRadius::same(4), fill.gamma_multiply(fade));
    icon.image(20.0, ink.gamma_multiply(fade)).paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(20.0)));
    let response = response.on_hover_text(tip);
    enabled && response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelAction {
    Leave,
    Status,
    Open,
    Camera,
    Share,
    Switch,
    Settings,
    Fullscreen,
    Sounds,
    Debug,
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_picture_of_a_share_is_a_small_jpeg() {
        let (width, height) = (1920u32, 1080u32);
        let rgba: Vec<u8> = (0..width * height).flat_map(|index| [(index % 251) as u8, (index / 7919 % 256) as u8, 90, 255]).collect();
        let image = super::thumb_of(&telinha_voice::video::Picture { width, height, rgba }).unwrap();
        let data = telinha_core::text::decode_base64(image.strip_prefix("data:image/jpeg;base64,").unwrap());
        let decoded = image::load_from_memory(&data).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (320, 180));
        // It must fit in one message of the room.
        assert!(image.len() < 120_000, "{} bytes", image.len());
    }

    use super::*;

    #[test]
    fn the_time_in_a_call_is_written_like_before() {
        assert_eq!(format_elapsed(0.0), "0:00");
        assert_eq!(format_elapsed(245.9), "4:05");
        assert_eq!(format_elapsed(4980.0), "1:23:00");
    }
}

impl TelinhaApp {
    /// Whether the camera of this app is sending in the call.
    pub(crate) fn camera_on(&self) -> bool {
        self.room.camera.is_some()
    }

    /// Where the frames of the camera go, made on first use.
    pub(crate) fn camera_feed(&mut self) -> CameraFeed {
        let context = self.context.clone();
        self.room.camera_feed.get_or_insert_with(|| CameraFeed::new(video_waker(context))).clone()
    }

    /// Turns the camera on or off in the call, and tells the room, as `setCameraOn`.
    pub fn set_camera(&mut self, on: bool) {
        if on == self.camera_on() {
            return;
        }
        if on {
            let Some(voice) = self.room.voice.clone() else { return };
            // A test of the camera in the settings holds it; the call takes it over.
            self.camera_test = None;
            let feed = self.camera_feed();
            feed.set_call(self.room.call.as_ref().map(|call| &call.call));
            let camera = Camera::start(self.prefs.camera.clone(), self.prefs.blur, feed.sink());
            self.room.camera = Some(LiveCamera { camera: Some(camera), space_id: voice.space_id, room_id: voice.room_id });
        } else {
            self.room.camera = None;
            if let Some(feed) = &self.room.camera_feed {
                feed.preview().clear();
            }
            self.room.textures.remove(SELF_VIDEO);
        }
        self.hub.update_voice(None, Some(on), None);
    }

    /// The camera running in the call, for the settings to change its blur.
    pub(crate) fn call_camera(&self) -> Option<&Camera> {
        self.room.camera.as_ref().and_then(|live| live.camera.as_ref())
    }

    /// Shows this picture as the camera of the person here, in place of a real camera. For the tests.
    pub fn camera_for_test(&mut self, width: u32, height: u32, rgba: Vec<u8>) {
        let Some(voice) = self.room.voice.clone() else { return };
        let feed = self.camera_feed();
        feed.preview().put(telinha_voice::video::Picture { width, height, rgba });
        self.room.camera = Some(LiveCamera { camera: None, space_id: voice.space_id, room_id: voice.room_id });
        self.hub.update_voice(None, Some(true), None);
    }

    /// Opens the camera again with the camera chosen now, when it is on.
    pub(crate) fn restart_camera(&mut self) {
        if self.camera_on() {
            self.room.camera = None;
            self.set_camera(true);
        }
    }

    /// The last frame of a video as a picture of the window, made again only when the frame changed.
    /// `drawn` is the size it is drawn at, in points, so a large video is made small before it
    /// reaches the window.
    pub(crate) fn video_texture(&mut self, context: &egui::Context, id: &str, store: &FrameStore, drawn: Vec2) -> Option<egui::load::SizedTexture> {
        let pixels = drawn * context.pixels_per_point();
        store.want_size(pixels.x.ceil() as u32, pixels.y.ceil() as u32, context.cumulative_pass_nr());
        let Some((serial, picture)) = store.latest() else {
            self.room.textures.remove(id);
            return None;
        };
        let image = |picture: &telinha_voice::video::Picture| egui::ColorImage::from_rgba_unmultiplied([picture.width as usize, picture.height as usize], &picture.rgba);
        match self.room.textures.get_mut(id) {
            Some((seen, texture)) => {
                if *seen != serial {
                    texture.set(image(&picture), egui::TextureOptions::LINEAR);
                    *seen = serial;
                }
                Some(egui::load::SizedTexture::from_handle(texture))
            }
            None => {
                let texture = context.load_texture(format!("video:{id}"), image(&picture), egui::TextureOptions::LINEAR);
                let sized = egui::load::SizedTexture::from_handle(&texture);
                self.room.textures.insert(id.to_owned(), (serial, texture));
                Some(sized)
            }
        }
    }
}
