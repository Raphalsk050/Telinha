//! Screen sharing inside a call: one engine sending the screen to whoever asks to watch, and one
//! engine per screen being watched. The apps only pass the connection codes around; the picture
//! goes straight from one engine to the other. Same protocol as `desktop/src/streams.js`.

use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::{Hub, HubEvent, LiveState};
use crate::engine::{
    EngineEvent, EngineSession, ShareAudio, ShareRequest, TargetKind, Targets, ascii_title, build_share_args,
    build_watch_args, list_targets, resolve_engine, stream_ports,
};
use crate::signaling::Peer;
use crate::text::{clean_name, now_ms};

const WATCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
const VIEWER_GRACE_MS: i64 = 15_000;
const RESTART_GRACE_MS: i64 = 8_000;
const MAX_SDP_LENGTH: usize = 60_000;
const MAX_THUMB_LENGTH: usize = 120_000;
const THUMB_PREFIX: &str = "data:image/jpeg;base64,";
/// A picture older than this no longer stands for the screen, as `THUMB_MAX_AGE_MS`.
const THUMB_MAX_AGE_MS: i64 = 90_000;
/// The viewer that stands for the person sharing, who sees their own screen inside the window.
pub const SELF_VIEWER: &str = "self";

/// What the person picked to share.
#[derive(Debug, Clone)]
pub struct ShareChoice {
    pub kind: TargetKind,
    pub handle: u64,
    pub name: String,
    pub audio: ShareAudio,
    pub fps: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutgoingPhase {
    Starting,
    Live,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncomingPhase {
    Requesting,
    Connecting,
    Live,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingView {
    pub space_id: String,
    pub room_id: String,
    pub phase: OutgoingPhase,
    pub width: i64,
    pub height: i64,
    pub target_name: String,
    /// Who is watching, and the state of each connection as the engine names it.
    pub viewers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingView {
    pub sharer_id: String,
    pub space_id: String,
    pub room_id: String,
    pub name: String,
    pub phase: IncomingPhase,
    pub fullscreen: bool,
    /// Watched inside the window, not in a window of the engine.
    pub embedded: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamsView {
    pub outgoing: Option<OutgoingView>,
    pub incoming: Vec<IncomingView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// The person here stopped watching.
    Stopped,
    /// The other side stopped sharing.
    Ended,
    Failed,
    /// The viewer window was closed.
    Closed,
}

struct Viewer {
    state: String,
    added_at: i64,
}

struct Outgoing {
    session: Arc<EngineSession>,
    serial: u64,
    space_id: String,
    room_id: String,
    phase: OutgoingPhase,
    width: i64,
    height: i64,
    target_name: String,
    error: Option<Value>,
    viewers: IndexMap<String, Viewer>,
}

struct Incoming {
    session: Option<Arc<EngineSession>>,
    serial: u64,
    space_id: String,
    room_id: String,
    name: String,
    phase: IncomingPhase,
    code: Option<String>,
    wants_code: bool,
    fullscreen: bool,
    stopping: bool,
    ended_by_sharer: bool,
    error: Option<Value>,
    requested_at: i64,
    embedded: bool,
    /// Where to watch next once the window of the engine closes: inside the window or not.
    switching: Option<bool>,
}

#[derive(Default)]
pub(super) struct Streams {
    outgoing: Option<Outgoing>,
    incoming: IndexMap<String, Incoming>,
    /// The last small picture each sharer sent of their screen, as JPEG, and when it came.
    thumbs: IndexMap<String, (Arc<[u8]>, i64)>,
    serial: u64,
}

impl Streams {
    fn next_serial(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }

    fn view(&self) -> StreamsView {
        StreamsView {
            outgoing: self.outgoing.as_ref().map(|outgoing| OutgoingView {
                space_id: outgoing.space_id.clone(),
                room_id: outgoing.room_id.clone(),
                phase: outgoing.phase,
                width: outgoing.width,
                height: outgoing.height,
                target_name: outgoing.target_name.clone(),
                viewers: outgoing.viewers.iter().filter(|(id, _)| id.as_str() != SELF_VIEWER).map(|(id, viewer)| (id.clone(), viewer.state.clone())).collect(),
            }),
            incoming: self
                .incoming
                .iter()
                .map(|(sharer_id, entry)| IncomingView {
                    sharer_id: sharer_id.clone(),
                    space_id: entry.space_id.clone(),
                    room_id: entry.room_id.clone(),
                    name: entry.name.clone(),
                    phase: entry.phase,
                    fullscreen: entry.fullscreen,
                    embedded: entry.embedded,
                })
                .collect(),
        }
    }
}

fn short_id(id: &str) -> &str {
    id.get(..4).unwrap_or(id)
}

/// How many addresses of each kind a description offers, for the log: with no `srflx` and no
/// `relay`, a viewer in another house cannot be reached.
fn candidate_summary(sdp: &str) -> String {
    let count = |kind: &str| sdp.lines().filter(|line| line.starts_with("a=candidate:") && line.contains(&format!(" typ {kind}"))).count();
    format!("{} host, {} srflx, {} relay", count("host"), count("srflx"), count("relay"))
}

fn limit(value: Option<i64>) -> i64 {
    value.filter(|value| *value > 0).unwrap_or(0)
}

/// The command that points a running share at another screen, window or capture card.
pub fn target_command(kind: TargetKind, handle: u64) -> Value {
    json!({ "command": "switch_target", "kind": kind.key(), "handle": handle })
}

pub fn quality_command(max_fps: Option<i64>, max_width: Option<i64>, max_height: Option<i64>, max_bitrate_kbps: Option<i64>) -> Value {
    json!({
        "command": "set_quality",
        "max_fps": limit(max_fps),
        "max_width": limit(max_width),
        "max_height": limit(max_height),
        "max_bitrate_kbps": limit(max_bitrate_kbps),
    })
}

pub fn audio_command(audio: &ShareAudio) -> Value {
    match audio {
        ShareAudio::System { .. } => json!({ "command": "set_audio", "scope": "system", "pid": 0 }),
        ShareAudio::Process { pid } => json!({ "command": "set_audio", "scope": "process", "pid": pid }),
        ShareAudio::Device { id } => json!({ "command": "set_audio", "scope": "device", "pid": 0, "device": id }),
        ShareAudio::None => json!({ "command": "set_audio", "scope": "none", "pid": 0 }),
    }
}

impl Hub {
    pub fn streams(&self) -> StreamsView {
        self.lock().streams.view()
    }

    /// The last small picture a sharer sent of their screen, while it is recent.
    pub fn stream_thumb(&self, sharer_id: &str) -> Option<Arc<[u8]>> {
        self.lock().streams.thumbs.get(sharer_id).filter(|(_, at)| now_ms() - at <= THUMB_MAX_AGE_MS).map(|(data, _)| Arc::clone(data))
    }

    /// Sends the room a small picture of the screen being shared, as a JPEG data address.
    pub fn publish_stream_thumb(&self, image: &str) -> bool {
        if image.len() > MAX_THUMB_LENGTH || !image.starts_with(THUMB_PREFIX) {
            return false;
        }
        let place = self.lock().streams.outgoing.as_ref().filter(|outgoing| outgoing.phase == OutgoingPhase::Live).map(|outgoing| (outgoing.space_id.clone(), outgoing.room_id.clone()));
        let Some((space_id, room_id)) = place else { return false };
        self.signaling.publish(&space_id, "stream-thumb", json!({ "roomId": room_id, "image": image }))
    }

    fn streams_changed(&self) {
        self.emit(HubEvent::Streams);
    }

    /// What the engine can share right now. Slow: it starts the engine and waits for the list.
    pub async fn share_targets(&self) -> Result<Targets, String> {
        let exe = resolve_engine().ok_or("não encontrei o motor de captura (telinha.exe)")?;
        list_targets(&exe).await
    }

    /// Starts sharing a screen in the call the person is in. Must be called from inside a tokio
    /// runtime. The error is a sentence ready to show.
    pub fn go_live(self: &Arc<Self>, choice: ShareChoice) -> Result<(), String> {
        let mut inner = self.lock();
        let voice = inner.voice.clone().ok_or("entre numa chamada antes de transmitir")?;
        if inner.streams.outgoing.is_some() {
            return Err("você já está transmitindo".into());
        }
        let exe = resolve_engine().ok_or("não encontrei o motor de captura (telinha.exe)")?;
        let audio = match choice.audio {
            ShareAudio::System { .. } => ShareAudio::System { exclude_pid: Some(std::process::id()) },
            other => other,
        };
        let request = ShareRequest {
            kind: choice.kind,
            handle: Some(choice.handle),
            index: None,
            audio,
            fps: choice.fps,
            multi: true,
            relay: inner.servers.get(&voice.space_id).and_then(|server| server.relay),
            ports: stream_ports(),
        };
        let args = build_share_args(&request)?;
        let (session, events) = EngineSession::start(&exe, &args)?;
        let serial = inner.streams.next_serial();
        inner.streams.outgoing = Some(Outgoing {
            session: Arc::new(session),
            serial,
            space_id: voice.space_id,
            room_id: voice.room_id,
            phase: OutgoingPhase::Starting,
            width: 0,
            height: 0,
            target_name: clean_name(&choice.name, "Tela"),
            error: None,
            viewers: IndexMap::new(),
        });
        drop(inner);
        self.pump_outgoing(serial, events);
        self.streams_changed();
        Ok(())
    }

    fn pump_outgoing(self: &Arc<Self>, serial: u64, mut events: mpsc::UnboundedReceiver<EngineEvent>) {
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                match event {
                    EngineEvent::Event(event) => hub.outgoing_event(serial, event),
                    EngineEvent::Log(line) => hub.engine_log("transmissao", &line),
                    EngineEvent::Exit { code, stopped } => {
                        hub.outgoing_exit(serial, code, stopped);
                        return;
                    }
                }
            }
        });
    }

    /// Lines of the engine worth keeping in the app's log.
    pub(super) fn engine_log(&self, source: &str, line: &str) {
        if line.contains("falha fatal") || line.contains("[error]") {
            self.log.error(&format!("telinha.exe ({source}): {line}"));
        }
    }

    fn outgoing_event(self: &Arc<Self>, serial: u64, event: Value) {
        self.emit(HubEvent::EngineEvent { source: "outgoing".into(), event: event.clone() });
        let mut inner = self.lock();
        let Some(outgoing) = inner.streams.outgoing.as_mut().filter(|outgoing| outgoing.serial == serial) else {
            return;
        };
        let kind = event.get("event").and_then(Value::as_str).unwrap_or("");
        let peer = event.get("peer").and_then(Value::as_str).unwrap_or("").to_owned();
        let known = outgoing.viewers.contains_key(&peer);
        let space_id = outgoing.space_id.clone();
        let room_id = outgoing.room_id.clone();
        let size = |name: &str| event.get(name).and_then(Value::as_i64).unwrap_or(0);
        let mut publish = None;
        let mut live = None;
        let mut changed = false;
        let mut own_offer = None;
        match kind {
            "ready" => {
                outgoing.phase = OutgoingPhase::Live;
                outgoing.width = size("width");
                outgoing.height = size("height");
                live = Some(LiveState { space_id: space_id.clone(), room_id: room_id.clone(), target_name: outgoing.target_name.clone() });
                // The person sharing watches their own screen too, as `watchOwnScreen`.
                if !outgoing.viewers.contains_key(SELF_VIEWER) {
                    outgoing.viewers.insert(SELF_VIEWER.to_owned(), Viewer { state: "preparing".into(), added_at: now_ms() });
                    outgoing.session.send_command(&json!({ "command": "add_peer", "peer": SELF_VIEWER, "format": "sdp" }));
                }
                changed = true;
            }
            "target" => {
                outgoing.width = size("width");
                outgoing.height = size("height");
                changed = true;
            }
            "code" if known && event.get("kind").and_then(Value::as_str) == Some("invite") => {
                outgoing.viewers[&peer].state = "inviting".into();
                publish = Some(("stream-offer", json!({ "to": peer, "roomId": room_id, "code": event.get("code") })));
                changed = true;
            }
            "offer" if known && peer == SELF_VIEWER => {
                outgoing.viewers[&peer].state = "inviting".into();
                if let Some(sdp) = event.get("sdp").and_then(Value::as_str) {
                    own_offer = Some(sdp.to_owned());
                }
            }
            "offer" if known && event.get("sdp").is_some_and(Value::is_string) => {
                outgoing.viewers[&peer].state = "inviting".into();
                publish = Some(("stream-offer", json!({ "to": peer, "roomId": room_id, "sdp": event.get("sdp") })));
                changed = true;
            }
            "code_rejected" if known => {
                outgoing.session.send_command(&json!({ "command": "remove_peer", "peer": peer }));
                if peer != SELF_VIEWER {
                    publish = Some(("stream-end", json!({ "to": peer, "roomId": room_id, "reason": "peer" })));
                }
            }
            "peer_state" if known => {
                outgoing.viewers[&peer].state = event.get("state").and_then(Value::as_str).unwrap_or("").to_owned();
                changed = true;
            }
            "peer_failed" if known && peer != SELF_VIEWER => {
                publish = Some(("stream-end", json!({ "to": peer, "roomId": room_id, "reason": "peer" })));
            }
            "peer_removed" => changed = outgoing.viewers.shift_remove(&peer).is_some(),
            "error" => outgoing.error = Some(event.clone()),
            _ => {}
        }
        drop(inner);
        if let Some(live) = live {
            self.set_live(Some(live));
        }
        if let Some((kind, body)) = publish {
            self.signaling.publish(&space_id, kind, body);
        }
        if let Some(sdp) = own_offer {
            self.emit(HubEvent::EmbeddedOffer { sharer_id: SELF_VIEWER.to_owned(), sdp });
        }
        if changed {
            self.streams_changed();
        }
    }

    fn outgoing_exit(&self, serial: u64, code: Option<i32>, stopped: bool) {
        let mut inner = self.lock();
        if inner.streams.outgoing.as_ref().is_none_or(|outgoing| outgoing.serial != serial) {
            return;
        }
        let Some(outgoing) = inner.streams.outgoing.take() else { return };
        drop(inner);
        self.signaling.publish(&outgoing.space_id, "stream-end", json!({ "roomId": outgoing.room_id, "reason": "stopped" }));
        self.set_live(None);
        if !stopped && (code.is_some_and(|code| code != 0) || outgoing.error.is_some()) {
            let error = outgoing.error.as_ref().map_or_else(String::new, |error| format!(" {error}"));
            self.log.error(&format!("a transmissao parou sozinha, codigo {code:?}{error}"));
        }
        self.emit(HubEvent::OutgoingEnded { code, stopped, error: outgoing.error });
        self.streams_changed();
    }

    pub fn stop_live(&self) {
        if let Some(outgoing) = &self.lock().streams.outgoing {
            outgoing.session.stop();
        }
    }

    /// Sends a command to the engine that is sharing: switch target, quality, sound.
    pub fn stream_command(&self, command: &Value) -> bool {
        let inner = self.lock();
        inner
            .streams
            .outgoing
            .as_ref()
            .is_some_and(|outgoing| outgoing.phase == OutgoingPhase::Live && outgoing.session.send_command(command))
    }

    /// Asks to watch the screen someone in the call is sharing. Must be called from inside a
    /// tokio runtime.
    pub fn watch(self: &Arc<Self>, sharer_id: &str) -> Result<(), String> {
        let mut inner = self.lock();
        let voice = inner.voice.clone().ok_or("entre na chamada para assistir")?;
        if sharer_id == self.instance_id() || inner.streams.incoming.contains_key(sharer_id) {
            return Ok(());
        }
        let sharer = self.signaling.peers(&voice.space_id).into_iter().find(|peer| {
            peer.id == sharer_id && peer.voice.as_ref().is_some_and(|peer_voice| peer_voice.room_id == voice.room_id && peer_voice.live)
        });
        let sharer = sharer.ok_or("essa transmissão já terminou")?;
        let name = match inner.contacts.get(&voice.space_id) {
            Some(contact) => contact.name,
            None if sharer.name.is_empty() => "Alguém".to_owned(),
            None => sharer.name,
        };
        let serial = inner.streams.next_serial();
        inner.streams.incoming.insert(
            sharer_id.to_owned(),
            Incoming {
                session: None,
                serial,
                space_id: voice.space_id.clone(),
                room_id: voice.room_id.clone(),
                name,
                phase: IncomingPhase::Requesting,
                code: None,
                wants_code: false,
                fullscreen: false,
                stopping: false,
                ended_by_sharer: false,
                error: None,
                requested_at: now_ms(),
                embedded: true,
                switching: None,
            },
        );
        drop(inner);
        if let Err(error) = self.request_stream(sharer_id) {
            self.lock().streams.incoming.shift_remove(sharer_id);
            return Err(error);
        }
        self.streams_changed();
        Ok(())
    }

    /// Asks who shares for the screen, inside the window or for a window of the engine, as
    /// `requestStream`. Whoever does not answer in time is given up on.
    fn request_stream(self: &Arc<Self>, sharer_id: &str) -> Result<(), String> {
        let mut inner = self.lock();
        let serial = inner.streams.next_serial();
        let Some(entry) = inner.streams.incoming.get_mut(sharer_id) else { return Err("essa transmissão já terminou".into()) };
        entry.serial = serial;
        entry.phase = IncomingPhase::Requesting;
        entry.requested_at = now_ms();
        entry.error = None;
        let format = if entry.embedded { "sdp" } else { "token" };
        let (space_id, room_id) = (entry.space_id.clone(), entry.room_id.clone());
        drop(inner);
        let request = json!({ "to": sharer_id, "roomId": room_id, "format": format });
        if !self.signaling.publish(&space_id, "stream-watch", request) {
            return Err("sem conexão com o servidor de contatos".into());
        }
        let hub = Arc::clone(self);
        let sharer_id = sharer_id.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(WATCH_TIMEOUT).await;
            let timed_out = {
                let mut inner = hub.lock();
                match inner.streams.incoming.get_mut(&sharer_id).filter(|entry| entry.serial == serial && entry.phase != IncomingPhase::Live) {
                    Some(entry) => {
                        entry.error = Some(json!({ "stage": "timeout" }));
                        true
                    }
                    None => false,
                }
            };
            if timed_out {
                hub.unwatch(&sharer_id);
            }
        });
        Ok(())
    }

    /// Moves a screen being watched to a window of its own, or back into the window of the
    /// app, as `switchMode`.
    pub fn set_stream_window(self: &Arc<Self>, sharer_id: &str, own_window: bool) {
        let mut inner = self.lock();
        let Some(entry) = inner.streams.incoming.get_mut(sharer_id) else { return };
        let embedded = !own_window;
        if entry.embedded == embedded && entry.switching.is_none() {
            return;
        }
        if let Some(session) = &entry.session {
            entry.switching = Some(embedded);
            session.stop();
            return;
        }
        entry.embedded = embedded;
        drop(inner);
        if self.request_stream(sharer_id).is_err() {
            self.drop_incoming(sharer_id);
            return;
        }
        self.streams_changed();
    }

    /// The answer of the window to the offer of a screen, sent back to who shares it.
    pub fn answer_embedded(&self, sharer_id: &str, sdp: &str) -> bool {
        if sdp.len() > MAX_SDP_LENGTH {
            return false;
        }
        let inner = self.lock();
        if sharer_id == SELF_VIEWER {
            return inner.streams.outgoing.as_ref().is_some_and(|outgoing| {
                outgoing.session.send_command(&json!({ "command": "peer_sdp_answer", "peer": SELF_VIEWER, "sdp": sdp }))
            });
        }
        let Some(entry) = inner.streams.incoming.get(sharer_id).filter(|entry| entry.embedded && !entry.stopping) else { return false };
        let (space_id, room_id) = (entry.space_id.clone(), entry.room_id.clone());
        drop(inner);
        self.signaling.publish(&space_id, "stream-answer", json!({ "to": sharer_id, "roomId": room_id, "sdp": sdp }))
    }

    /// The connection of a screen watched inside the window came up, or failed.
    pub fn embedded_state(&self, sharer_id: &str, live: bool) {
        let mut inner = self.lock();
        let Some(entry) = inner.streams.incoming.get_mut(sharer_id).filter(|entry| entry.embedded) else { return };
        if live {
            if entry.phase != IncomingPhase::Live {
                entry.phase = IncomingPhase::Live;
                drop(inner);
                self.streams_changed();
            }
        } else {
            entry.error = Some(json!({ "stage": "network" }));
            drop(inner);
            self.unwatch(sharer_id);
        }
    }

    pub fn unwatch(&self, sharer_id: &str) {
        let mut inner = self.lock();
        let Some(entry) = inner.streams.incoming.get_mut(sharer_id) else { return };
        entry.stopping = true;
        match &entry.session {
            Some(session) => session.stop(),
            None => {
                drop(inner);
                self.drop_incoming(sharer_id);
            }
        }
    }

    /// Forgets a screen that was never opened in a viewer.
    fn drop_incoming(&self, sharer_id: &str) {
        let Some(entry) = self.lock().streams.incoming.shift_remove(sharer_id) else { return };
        if !entry.ended_by_sharer {
            self.signaling.publish(&entry.space_id, "stream-leave", json!({ "to": sharer_id, "roomId": entry.room_id }));
        }
        let reason = match (entry.ended_by_sharer, entry.error.is_some()) {
            (true, _) => EndReason::Ended,
            (false, true) => EndReason::Failed,
            (false, false) => EndReason::Stopped,
        };
        self.emit(HubEvent::IncomingEnded { sharer_id: sharer_id.to_owned(), name: entry.name, reason, error: entry.error });
        self.streams_changed();
    }

    fn start_viewer(self: &Arc<Self>, sharer_id: &str) {
        let Some(exe) = resolve_engine() else { return };
        let mut inner = self.lock();
        let Some(entry) = inner.streams.incoming.get_mut(sharer_id) else { return };
        let title = ascii_title(&format!("Telinha - {}", entry.name));
        let mut args = build_watch_args(false, true, &title, stream_ports());
        // For the tests: the viewer runs without a window and without sound.
        if std::env::var_os("TELINHA_HEADLESS_VIEWER").is_some() {
            args.extend(["--headless".to_owned(), "--no-audio".to_owned()]);
        }
        let Ok((session, events)) = EngineSession::start(&exe, &args) else {
            return;
        };
        entry.session = Some(Arc::new(session));
        entry.phase = IncomingPhase::Connecting;
        let serial = entry.serial;
        drop(inner);

        let hub = Arc::clone(self);
        let sharer_id = sharer_id.to_owned();
        let mut events = events;
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                match event {
                    EngineEvent::Event(event) => hub.incoming_event(&sharer_id, serial, event),
                    EngineEvent::Log(line) => hub.engine_log("assistindo", &line),
                    EngineEvent::Exit { code, .. } => {
                        hub.incoming_exit(&sharer_id, serial, code);
                        return;
                    }
                }
            }
        });
        self.streams_changed();
    }

    fn deliver_code(entry: &mut Incoming) {
        if !entry.wants_code {
            return;
        }
        let (Some(code), Some(session)) = (entry.code.take(), entry.session.as_ref()) else { return };
        entry.wants_code = false;
        session.submit_code(&code);
    }

    fn incoming_event(&self, sharer_id: &str, serial: u64, event: Value) {
        self.emit(HubEvent::EngineEvent { source: sharer_id.to_owned(), event: event.clone() });
        let mut inner = self.lock();
        let Some(entry) = inner.streams.incoming.get_mut(sharer_id).filter(|entry| entry.serial == serial) else {
            return;
        };
        let mut publish = None;
        let mut changed = false;
        match event.get("event").and_then(Value::as_str).unwrap_or("") {
            "need_code" => {
                entry.wants_code = true;
                Self::deliver_code(entry);
            }
            "code" if event.get("kind").and_then(Value::as_str) == Some("answer") => {
                publish = Some((entry.space_id.clone(), json!({ "to": sharer_id, "roomId": entry.room_id, "code": event.get("code") })));
            }
            "code_rejected" => {
                entry.error = Some(json!({ "stage": "code", "message": event.get("message") }));
                if let Some(session) = &entry.session {
                    session.stop();
                }
            }
            "state" if event.get("state").and_then(Value::as_str) == Some("Connected") && entry.phase != IncomingPhase::Live => {
                entry.phase = IncomingPhase::Live;
                changed = true;
            }
            "fullscreen" => {
                entry.fullscreen = event.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                changed = true;
            }
            "error" => entry.error = Some(event.clone()),
            _ => {}
        }
        drop(inner);
        if let Some((space_id, body)) = publish {
            self.signaling.publish(&space_id, "stream-answer", body);
        }
        if changed {
            self.streams_changed();
        }
    }

    fn incoming_exit(self: &Arc<Self>, sharer_id: &str, serial: u64, code: Option<i32>) {
        let mut inner = self.lock();
        let Some(entry) = inner.streams.incoming.get_mut(sharer_id).filter(|entry| entry.serial == serial) else {
            return;
        };
        entry.session = None;
        entry.code = None;
        entry.wants_code = false;
        entry.fullscreen = false;
        // Closing the window of the engine brings the screen back into the app, as streams.js did.
        let closed_by_user = !entry.stopping && !entry.ended_by_sharer && entry.error.is_none() && code == Some(0);
        if let Some(embedded) = entry.switching.take().or(closed_by_user.then_some(true)) {
            entry.stopping = false;
            entry.embedded = embedded;
            drop(inner);
            if self.request_stream(sharer_id).is_ok() {
                self.streams_changed();
                return;
            }
            inner = self.lock();
        }
        let Some(entry) = inner.streams.incoming.shift_remove(sharer_id) else { return };
        drop(inner);
        let reason = if entry.ended_by_sharer {
            EndReason::Ended
        } else if entry.error.is_some() {
            EndReason::Failed
        } else if entry.stopping {
            EndReason::Stopped
        } else if code != Some(0) {
            EndReason::Failed
        } else {
            EndReason::Closed
        };
        if !entry.ended_by_sharer {
            self.signaling.publish(&entry.space_id, "stream-leave", json!({ "to": sharer_id, "roomId": entry.room_id }));
        }
        if reason == EndReason::Failed {
            self.log.error(&format!("assistir a transmissao falhou, codigo {code:?} {:?}", entry.error));
        }
        self.emit(HubEvent::IncomingEnded { sharer_id: sharer_id.to_owned(), name: entry.name, reason, error: entry.error });
        self.streams_changed();
    }

    pub fn set_stream_volume(&self, sharer_id: &str, volume: i64) -> bool {
        let inner = self.lock();
        let Some(entry) = inner.streams.incoming.get(sharer_id).filter(|entry| entry.phase == IncomingPhase::Live) else {
            return false;
        };
        let command = json!({ "command": "set_volume", "volume": volume.clamp(0, 200) });
        entry.session.as_ref().is_some_and(|session| session.send_command(&command))
    }

    pub fn set_stream_fullscreen(&self, sharer_id: &str, enabled: bool) -> bool {
        let inner = self.lock();
        let Some(entry) = inner.streams.incoming.get(sharer_id).filter(|entry| entry.phase == IncomingPhase::Live) else {
            return false;
        };
        entry.session.as_ref().is_some_and(|session| session.send_command(&json!({ "command": "set_fullscreen", "enabled": enabled })))
    }

    /// A message of the stream protocol that arrived from someone in the space.
    pub(super) fn handle_stream_message(self: &Arc<Self>, space_id: &str, message: &Value) {
        let text = |name: &str| message.get(name).and_then(Value::as_str);
        let Some(from) = text("from").map(str::to_owned) else { return };
        if text("to").is_some_and(|to| to != self.instance_id()) || from == SELF_VIEWER {
            return;
        }
        let room_id = text("roomId").unwrap_or("").to_owned();
        match text("type").unwrap_or("") {
            "stream-watch" => {
                let mut inner = self.lock();
                let live = inner.streams.outgoing.as_mut().filter(|outgoing| {
                    outgoing.phase == OutgoingPhase::Live && outgoing.space_id == space_id && outgoing.room_id == room_id
                });
                let Some(outgoing) = live else {
                    drop(inner);
                    self.signaling.publish(space_id, "stream-end", json!({ "to": from, "roomId": room_id, "reason": "stopped" }));
                    return;
                };
                let format = if text("format") == Some("sdp") { "sdp" } else { "token" };
                outgoing.viewers.insert(from.clone(), Viewer { state: "preparing".into(), added_at: now_ms() });
                outgoing.session.send_command(&json!({ "command": "add_peer", "peer": from, "format": format }));
                drop(inner);
                self.streams_changed();
            }
            "stream-offer" => {
                let mut inner = self.lock();
                let Some(entry) = inner.streams.incoming.get_mut(&from).filter(|entry| entry.space_id == space_id && !entry.stopping) else {
                    return;
                };
                if entry.embedded {
                    let Some(sdp) = text("sdp").filter(|sdp| sdp.len() <= MAX_SDP_LENGTH) else { return };
                    entry.phase = IncomingPhase::Connecting;
                    drop(inner);
                    self.emit(HubEvent::EmbeddedOffer { sharer_id: from.clone(), sdp: sdp.to_owned() });
                    self.streams_changed();
                    return;
                }
                let Some(code) = text("code") else { return };
                entry.code = Some(code.to_owned());
                let start = entry.session.is_none();
                if !start {
                    Self::deliver_code(entry);
                }
                drop(inner);
                if start {
                    self.start_viewer(&from);
                }
            }
            "stream-answer" => {
                let mut inner = self.lock();
                let Some(outgoing) = inner.streams.outgoing.as_mut().filter(|outgoing| outgoing.space_id == space_id) else {
                    return;
                };
                if !outgoing.viewers.contains_key(&from) {
                    return;
                }
                let command = match (text("sdp").filter(|sdp| sdp.len() <= MAX_SDP_LENGTH), text("code")) {
                    (Some(sdp), _) => {
                        self.log.info(&format!("transmissao: resposta de {}, {}", short_id(&from), candidate_summary(sdp)));
                        json!({ "command": "peer_sdp_answer", "peer": from, "sdp": sdp })
                    }
                    (None, Some(code)) => json!({ "command": "peer_answer", "peer": from, "code": code }),
                    (None, None) => return,
                };
                outgoing.session.send_command(&command);
                outgoing.viewers[&from].state = "connecting".into();
                drop(inner);
                self.streams_changed();
            }
            "stream-leave" => {
                let inner = self.lock();
                if let Some(outgoing) = inner.streams.outgoing.as_ref().filter(|outgoing| outgoing.space_id == space_id && outgoing.viewers.contains_key(&from)) {
                    outgoing.session.send_command(&json!({ "command": "remove_peer", "peer": from }));
                }
            }
            "stream-thumb" => {
                let Some(image) = text("image").filter(|image| image.len() <= MAX_THUMB_LENGTH).and_then(|image| image.strip_prefix(THUMB_PREFIX)) else {
                    return;
                };
                let data = crate::text::decode_base64(image);
                if !crate::avatars::is_jpeg(&data) {
                    return;
                }
                self.lock().streams.thumbs.insert(from.clone(), (data.into(), now_ms()));
                self.emit(HubEvent::StreamThumb { sharer_id: from });
            }
            "stream-end" => {
                let mut inner = self.lock();
                let Some(entry) = inner.streams.incoming.get_mut(&from).filter(|entry| entry.space_id == space_id) else {
                    return;
                };
                // The end of one attempt may arrive after a new request was already made.
                if text("reason") == Some("peer") && now_ms() - entry.requested_at < RESTART_GRACE_MS {
                    return;
                }
                entry.ended_by_sharer = true;
                drop(inner);
                self.unwatch(&from);
            }
            _ => {}
        }
    }

    /// Drops what no longer makes sense when the people in the room change: screens whose
    /// sharer stopped, and viewers who left.
    pub(super) fn reconcile_streams(&self, space_id: &str, room_id: &str, peers: &[Peer]) {
        let in_room: Vec<&Peer> =
            peers.iter().filter(|peer| peer.voice.as_ref().is_some_and(|voice| voice.room_id == room_id)).collect();
        let mut ended = Vec::new();
        {
            let mut inner = self.lock();
            for (sharer_id, entry) in inner.streams.incoming.iter_mut().filter(|(_, entry)| entry.space_id == space_id) {
                let live = in_room.iter().any(|peer| &peer.id == sharer_id && peer.voice.as_ref().is_some_and(|voice| voice.live));
                if !live {
                    entry.ended_by_sharer = true;
                    ended.push(sharer_id.clone());
                }
            }
            if let Some(outgoing) = inner.streams.outgoing.as_ref().filter(|outgoing| outgoing.space_id == space_id) {
                let now = now_ms();
                for (viewer_id, viewer) in &outgoing.viewers {
                    if viewer_id != SELF_VIEWER && !in_room.iter().any(|peer| &peer.id == viewer_id) && now - viewer.added_at > VIEWER_GRACE_MS {
                        outgoing.session.send_command(&json!({ "command": "remove_peer", "peer": viewer_id }));
                    }
                }
            }
            let gone: Vec<String> = inner.streams.thumbs.keys().filter(|id| !in_room.iter().any(|peer| &&peer.id == id)).cloned().collect();
            for id in gone {
                inner.streams.thumbs.shift_remove(&id);
            }
        }
        for sharer_id in ended {
            self.unwatch(&sharer_id);
        }
    }

    /// Stops sharing and watching, as leaving the call does.
    pub(super) fn leave_streams(&self) {
        self.stop_live();
        let watching: Vec<String> = self.lock().streams.incoming.keys().cloned().collect();
        for sharer_id in watching {
            self.unwatch(&sharer_id);
        }
    }
}
