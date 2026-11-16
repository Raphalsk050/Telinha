//! How the apps find and talk to each other: public MQTT message servers, where each space (a
//! contact or a server) is one topic and everything on it is sealed with the space's key.
//!
//! The app stays connected to every message server at once. Whoever opens the app finds the
//! others through the first server that answers, and two people meet even when each one only
//! reaches a different server. Every message carries an id, and the copy that arrives through
//! another server is dropped. Same protocol as `desktop/src/signaling.js`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use indexmap::IndexMap;
use rumqttc::{AsyncClient, Event, MqttOptions, Outgoing, Packet, QoS, Transport};
use serde_json::{Map, Value, json};
use tokio::sync::{Notify, mpsc};

use crate::disk::number_of;
use crate::envelope::{open_message, seal_message};
use crate::profile::{Style, clean_style};
use crate::servers::is_member_id;
use crate::text::{new_uuid, now_ms, truncate_utf16};

pub const TOPIC_PREFIX: &str = "telinha/v1/";
const PRESENCE_INTERVAL: Duration = Duration::from_secs(25);
const PRESENCE_TTL_MS: i64 = 70_000;
const RETRY_DELAY_MS: u64 = 5_000;
const MAX_RETRY_DELAY_MS: u64 = 60_000;
const RECONNECT_DELAY_MS: u64 = 1_000;
/// Covers the clock slack `open_message` accepts, so the copy that arrives through another
/// server never passes as a new message.
const SEEN_TTL_MS: i64 = 6 * 60 * 1000;
const MAX_SEEN: usize = 5000;
const WIDE_PAYLOAD_BYTES: usize = 16 * 1024;
const MAX_PACKET_BYTES: usize = 4 * 1024 * 1024;
const REQUEST_QUEUE: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broker {
    pub url: String,
    /// The largest message this server takes before it drops the connection.
    pub max_bytes: usize,
}

impl Broker {
    pub fn new(url: &str) -> Self {
        Self { url: url.to_owned(), max_bytes: usize::MAX }
    }
}

/// shiftr drops the connection on a large message: 60 KB went through, 150 KB did not. It comes
/// last so large messages prefer the others when all of them reach the people.
pub fn default_brokers() -> Vec<Broker> {
    vec![
        Broker::new("wss://broker.hivemq.com:8884/mqtt"),
        Broker::new("wss://broker.emqx.io:8084/mqtt"),
        Broker::new("wss://test.mosquitto.org:8081"),
        Broker { url: "wss://public:public@public.cloud.shiftr.io:443".into(), max_bytes: 60 * 1024 },
    ]
}

/// Ids of app instances and of voice rooms.
pub fn is_peer_id(text: &str) -> bool {
    (1..=64).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voice {
    pub room_id: String,
    pub mic: bool,
    pub camera: bool,
    pub deaf: bool,
    pub live: bool,
    pub live_name: String,
    /// The version of the call negotiation the peer speaks.
    pub version: i64,
}

/// Someone who is online in a space, as their app describes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// Names this run of their app. It changes every time the app opens.
    pub id: String,
    pub name: String,
    /// Names the person, and stays the same across runs.
    pub member_id: Option<String>,
    pub voice: Option<Voice>,
    pub style: Option<Style>,
}

fn describe_peer(id: &str, message: &Value) -> Peer {
    let text = |value: Option<&Value>, max: usize| {
        value.and_then(Value::as_str).map_or_else(String::new, |text| truncate_utf16(text, max).to_owned())
    };
    let voice = message.get("voice").filter(|voice| voice.is_object()).and_then(|voice| {
        let room_id = match voice.get("roomId")? {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            _ => return None,
        };
        if !is_peer_id(&room_id) {
            return None;
        }
        let flag = |name: &str| voice.get(name).and_then(Value::as_bool);
        let version = voice.get("v").and_then(Value::as_i64).filter(|version| *version > 0).unwrap_or(1);
        Some(Voice {
            room_id,
            mic: flag("mic") != Some(false),
            camera: flag("camera") == Some(true),
            deaf: flag("deaf") == Some(true),
            live: flag("live") == Some(true),
            live_name: text(voice.get("liveName"), 80),
            version,
        })
    });
    let style = clean_style(message.get("style"));
    Peer {
        id: id.to_owned(),
        name: text(message.get("name"), 40),
        member_id: message.get("memberId").and_then(Value::as_str).filter(|id| is_member_id(id)).map(str::to_owned),
        voice,
        style: style.is_set().then_some(style),
    }
}

/// What the signaling needs to know about who is using the app. Never called while the
/// signaling holds its own lock, so an implementation may lock whatever it needs.
pub trait Identity: Send + Sync {
    /// Every space this app listens to.
    fn space_ids(&self) -> Vec<String>;
    fn key_for(&self, space_id: &str) -> Option<[u8; 32]>;
    fn profile_name(&self) -> String;
    /// What goes in the presence of a space besides the name: who the person is, the call they
    /// are in, and for a server what this app knows about it.
    fn presence_for(&self, space_id: &str) -> Map<String, Value>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum SignalEvent {
    /// Anything that is not presence, already opened. `from` is the id of the sender's app.
    Message { space_id: String, message: Value },
    /// The list of who is online in a space changed.
    Peers { space_id: String, peers: Vec<Peer> },
    /// A space went from nobody online to somebody, or back.
    Online { space_id: String, online: bool },
    PeerName { space_id: String, name: String },
    /// A presence arrived, with everything it carries, for whoever reads the extra fields.
    PeerPresence { space_id: String, peer_id: String, message: Value },
    Status(Status),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub connected: bool,
    pub brokers: Vec<String>,
}

type AckCallback = Box<dyn FnOnce(bool) + Send>;

/// One answer for a message sent through several servers: the first server that confirms is
/// enough, and it only fails when every one of them failed.
struct SharedAck {
    callback: Option<AckCallback>,
    failures: usize,
    targets: usize,
}

#[derive(Clone)]
struct Ack(Arc<Mutex<SharedAck>>);

impl Ack {
    fn new(callback: AckCallback, targets: usize) -> Self {
        Self(Arc::new(Mutex::new(SharedAck { callback: Some(callback), failures: 0, targets })))
    }

    fn settle(&self, delivered: bool) {
        let callback = {
            let mut shared = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if !delivered {
                shared.failures += 1;
                if shared.failures < shared.targets {
                    return;
                }
            }
            shared.callback.take()
        };
        if let Some(callback) = callback {
            callback(delivered);
        }
    }
}

struct Link {
    broker: Broker,
    client: Option<AsyncClient>,
    connected: bool,
    /// Publishes handed to the client and not written yet, in order.
    queued: VecDeque<Option<Ack>>,
    /// Publishes written and waiting for the server to confirm, by packet id.
    waiting: HashMap<u16, Ack>,
}

impl Link {
    fn fail_all(&mut self) -> Vec<Ack> {
        self.queued.drain(..).flatten().chain(self.waiting.drain().map(|(_, ack)| ack)).collect()
    }
}

struct PresenceEntry {
    seen_at: i64,
    info: Peer,
}

#[derive(Default)]
struct State {
    links: Vec<Link>,
    presence: HashMap<String, IndexMap<String, PresenceEntry>>,
    /// Through which servers each peer of each space has been heard, and when.
    routes: HashMap<String, HashMap<String, HashMap<String, i64>>>,
    seen: IndexMap<String, i64>,
    /// Instances replaced by a newer one of the same person: an app that was restarted. Their
    /// late or repeated messages must not bring the old copy back.
    retired: IndexMap<String, i64>,
    /// Where everything sent goes instead of the servers, when a test asked for it.
    tap: Option<mpsc::UnboundedSender<(String, Value)>>,
}

impl State {
    fn peers(&self, space_id: &str, now: i64) -> Vec<Peer> {
        self.presence
            .get(space_id)
            .into_iter()
            .flat_map(|peers| peers.values())
            .filter(|entry| now - entry.seen_at < PRESENCE_TTL_MS)
            .map(|entry| entry.info.clone())
            .collect()
    }

    fn status(&self) -> Status {
        let brokers: Vec<String> =
            self.links.iter().filter(|link| link.connected).map(|link| link.broker.url.clone()).collect();
        Status { connected: !brokers.is_empty(), brokers }
    }

    /// Old versions send no id, but they only use one server and never arrive twice.
    fn first_sight(&mut self, message: &Value, now: i64) -> bool {
        let Some(mid) = message.get("mid").and_then(Value::as_str) else {
            return true;
        };
        if self.seen.contains_key(mid) {
            return false;
        }
        self.seen.insert(mid.to_owned(), now);
        if self.seen.len() > MAX_SEEN {
            self.seen.shift_remove_index(0);
        }
        true
    }

    fn expire_seen(&mut self, now: i64) {
        let old = self.seen.values().take_while(|at| now - **at >= SEEN_TTL_MS).count();
        self.seen.drain(..old);
    }

    /// A small message goes through every server. A large one (a piece of a file, a picture, a
    /// thumbnail) goes only through the servers where the people online are, so the upload is
    /// not paid three times.
    fn route_for(&self, space_id: &str, size: usize, open: Vec<usize>, now: i64) -> Vec<usize> {
        if size <= WIDE_PAYLOAD_BYTES || open.len() < 2 {
            return open;
        }
        let routes = self.routes.get(space_id);
        let mut left: Vec<HashSet<&str>> = self
            .peers(space_id, now)
            .iter()
            .map(|peer| {
                routes
                    .and_then(|routes| routes.get(&peer.id))
                    .into_iter()
                    .flatten()
                    .filter(|(_, at)| now - **at < PRESENCE_TTL_MS)
                    .map(|(url, _)| url.as_str())
                    .collect()
            })
            .collect();
        if left.is_empty() {
            return vec![open[0]];
        }
        let mut chosen = Vec::new();
        while !left.is_empty() {
            let reach = |index: usize| left.iter().filter(|urls| urls.contains(self.links[index].broker.url.as_str())).count();
            // The first of the servers that reach the most people, like the JavaScript app picks.
            let best = open.iter().copied().rev().max_by_key(|index| reach(*index)).filter(|index| reach(*index) > 0);
            let Some(best) = best else {
                return open;
            };
            chosen.push(best);
            let url = self.links[best].broker.url.clone();
            left.retain(|urls| !urls.contains(url.as_str()));
        }
        chosen
    }
}

pub struct Signaling {
    identity: Arc<dyn Identity>,
    instance_id: String,
    events: mpsc::UnboundedSender<SignalEvent>,
    state: Mutex<State>,
    stopped: AtomicBool,
    stop: Notify,
}

impl Signaling {
    pub fn new(
        identity: Arc<dyn Identity>,
        brokers: Vec<Broker>,
        events: mpsc::UnboundedSender<SignalEvent>,
    ) -> Arc<Self> {
        let links = brokers
            .into_iter()
            .map(|broker| Link { broker, client: None, connected: false, queued: VecDeque::new(), waiting: HashMap::new() })
            .collect();
        Arc::new(Self {
            identity,
            instance_id: new_uuid(),
            events,
            state: Mutex::new(State { links, ..State::default() }),
            stopped: AtomicBool::new(true),
            stop: Notify::new(),
        })
    }

    /// Names this run of the app to the others. A message from this id is our own echo.
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn emit(&self, event: SignalEvent) {
        let _ = self.events.send(event);
    }

    /// Connects to every message server and keeps the presence going. Must be called from inside
    /// a tokio runtime.
    pub fn start(self: &Arc<Self>) {
        self.stopped.store(false, Ordering::SeqCst);
        let count = self.lock().links.len();
        for index in 0..count {
            tokio::spawn(Arc::clone(self).run_link(index));
        }
        let signaling = Arc::clone(self);
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(PRESENCE_INTERVAL);
            timer.tick().await;
            loop {
                tokio::select! {
                    _ = timer.tick() => {}
                    _ = signaling.stop.notified() => return,
                }
                if signaling.stopped.load(Ordering::SeqCst) {
                    return;
                }
                signaling.publish_presence(true, None);
                signaling.expire(now_ms());
            }
        });
    }

    async fn run_link(self: Arc<Self>, index: usize) {
        let broker = self.lock().links[index].broker.clone();
        let mut failures: u32 = 0;
        while !self.stopped.load(Ordering::SeqCst) {
            let Some(options) = mqtt_options(&broker.url, &self.instance_id) else {
                return;
            };
            let (client, mut eventloop) = AsyncClient::new(options, REQUEST_QUEUE);
            eventloop.network_options.set_connection_timeout(8);
            let mut was_connected = false;
            loop {
                let event = tokio::select! {
                    event = eventloop.poll() => event,
                    _ = self.stop.notified() => break,
                };
                if self.stopped.load(Ordering::SeqCst) {
                    break;
                }
                match event {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        was_connected = true;
                        self.connected(index, client.clone());
                    }
                    Ok(Event::Incoming(Packet::Publish(publish))) => {
                        if let Ok(payload) = std::str::from_utf8(&publish.payload) {
                            self.handle_message(&publish.topic, payload, index);
                        }
                    }
                    Ok(Event::Incoming(Packet::PubAck(ack))) => {
                        let waiting = self.lock().links[index].waiting.remove(&ack.pkid);
                        if let Some(ack) = waiting {
                            ack.settle(true);
                        }
                    }
                    Ok(Event::Outgoing(Outgoing::Publish(pkid))) => {
                        let mut state = self.lock();
                        let link = &mut state.links[index];
                        if let Some(Some(ack)) = link.queued.pop_front() {
                            if pkid == 0 {
                                drop(state);
                                ack.settle(true);
                            } else {
                                link.waiting.insert(pkid, ack);
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }

            let failed = {
                let mut state = self.lock();
                let link = &mut state.links[index];
                link.client = None;
                link.connected = false;
                link.fail_all()
            };
            for ack in failed {
                ack.settle(false);
            }
            if was_connected {
                self.emit(SignalEvent::Status(self.status()));
            }
            if self.stopped.load(Ordering::SeqCst) {
                let _ = client.try_disconnect();
                // Lets the goodbye and the disconnect leave before the connection is dropped.
                let _ = tokio::time::timeout(Duration::from_millis(600), async {
                    while eventloop.poll().await.is_ok() {}
                })
                .await;
                return;
            }

            failures = if was_connected { 0 } else { failures + 1 };
            let delay = if was_connected {
                RECONNECT_DELAY_MS
            } else {
                MAX_RETRY_DELAY_MS.min(RETRY_DELAY_MS.saturating_mul(1 << (failures - 1).min(10)))
            };
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(delay)) => {}
                _ = self.stop.notified() => return,
            }
        }
    }

    fn connected(&self, index: usize, client: AsyncClient) {
        let topics: Vec<rumqttc::SubscribeFilter> = self
            .identity
            .space_ids()
            .into_iter()
            .map(|id| rumqttc::SubscribeFilter::new(format!("{TOPIC_PREFIX}{id}"), QoS::AtLeastOnce))
            .collect();
        if !topics.is_empty() {
            let _ = client.try_subscribe_many(topics);
        }
        {
            let mut state = self.lock();
            let link = &mut state.links[index];
            link.client = Some(client);
            link.connected = true;
        }
        self.send_presence(true, None, Some(index));
        self.emit(SignalEvent::Status(self.status()));
    }

    pub fn add_space(&self, space_id: &str) {
        let clients = self.open_clients();
        for client in &clients {
            let _ = client.try_subscribe(format!("{TOPIC_PREFIX}{space_id}"), QoS::AtLeastOnce);
        }
        if !clients.is_empty() {
            self.publish_presence(true, Some(space_id));
        }
    }

    /// Says goodbye in a space and stops listening to it. Call before the space leaves the
    /// stores, while its key is still there.
    pub fn remove_space(&self, space_id: &str) {
        let body = json!({ "online": false, "name": self.identity.profile_name() });
        self.send(space_id, "presence", body, None, None);
        for client in self.open_clients() {
            let _ = client.try_unsubscribe(format!("{TOPIC_PREFIX}{space_id}"));
        }
        let mut state = self.lock();
        state.presence.remove(space_id);
        state.routes.remove(space_id);
    }

    fn open_clients(&self) -> Vec<AsyncClient> {
        self.lock().links.iter().filter(|link| link.connected).filter_map(|link| link.client.clone()).collect()
    }

    /// Sends a message to everyone in a space. False when it could not even leave: no server
    /// connected, or none that takes a message this large.
    pub fn publish(&self, space_id: &str, kind: &str, body: Value) -> bool {
        self.send(space_id, kind, body, None, None)
    }

    /// Like `publish`, and tells afterwards whether some server confirmed the message.
    pub fn publish_acked(
        &self,
        space_id: &str,
        kind: &str,
        body: Value,
        on_acked: impl FnOnce(bool) + Send + 'static,
    ) -> bool {
        self.send(space_id, kind, body, Some(Box::new(on_acked)), None)
    }

    fn send(&self, space_id: &str, kind: &str, body: Value, on_acked: Option<AckCallback>, only: Option<usize>) -> bool {
        let Some(key) = self.identity.key_for(space_id) else {
            return false;
        };
        let now = now_ms();
        let mut message = match body {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        message.insert("type".into(), kind.into());
        message.insert("from".into(), self.instance_id.clone().into());
        message.insert("at".into(), now.into());
        message.insert("mid".into(), new_uuid().into());
        let tap = self.lock().tap.clone();
        if let Some(tap) = tap {
            let sent = tap.send((space_id.to_owned(), Value::Object(message))).is_ok();
            if let Some(on_acked) = on_acked {
                on_acked(sent);
            }
            return sent;
        }
        let payload = seal_message(&key, &Value::Object(message));
        let topic = format!("{TOPIC_PREFIX}{space_id}");

        let mut state = self.lock();
        let open: Vec<usize> = (0..state.links.len())
            .filter(|index| state.links[*index].connected && state.links[*index].client.is_some())
            .filter(|index| only.is_none_or(|only| only == *index))
            .filter(|index| payload.len() <= state.links[*index].broker.max_bytes)
            .collect();
        if open.is_empty() {
            return false;
        }
        let targets = if only.is_some() { open } else { state.route_for(space_id, payload.len(), open, now) };
        let ack = on_acked.map(|callback| Ack::new(callback, targets.len()));
        let mut refused = 0;
        for index in &targets {
            let link = &mut state.links[*index];
            let client = link.client.as_ref().expect("an open link has a client");
            if client.try_publish(topic.clone(), QoS::AtLeastOnce, false, payload.clone()).is_ok() {
                link.queued.push_back(ack.clone());
            } else {
                refused += 1;
            }
        }
        drop(state);
        if let Some(ack) = &ack {
            for _ in 0..refused {
                ack.settle(false);
            }
        }
        refused < targets.len()
    }

    /// Announces this app in one space or in all of them.
    pub fn publish_presence(&self, online: bool, space_id: Option<&str>) {
        self.send_presence(online, space_id, None);
    }

    fn send_presence(&self, online: bool, space_id: Option<&str>, only: Option<usize>) {
        let ids = space_id.map_or_else(|| self.identity.space_ids(), |id| vec![id.to_owned()]);
        let name = self.identity.profile_name();
        for id in ids {
            let mut body = Map::new();
            body.insert("name".into(), name.clone().into());
            if online {
                body.extend(self.identity.presence_for(&id));
            }
            body.insert("online".into(), online.into());
            self.send(&id, "presence", Value::Object(body), None, only);
        }
    }

    fn handle_message(&self, topic: &str, payload: &str, link: usize) {
        let Some(space_id) = topic.strip_prefix(TOPIC_PREFIX) else {
            return;
        };
        let Some(key) = self.identity.key_for(space_id) else {
            return;
        };
        let now = now_ms();
        let Some(message) = open_message(&key, payload, now) else {
            return;
        };
        let Some(from) = message.get("from").and_then(Value::as_str).map(str::to_owned) else {
            return;
        };
        if from == self.instance_id {
            return;
        }

        let mut events = Vec::new();
        let mut answer = false;
        {
            let mut state = self.lock();
            let url = state.links[link].broker.url.clone();
            state.routes.entry(space_id.to_owned()).or_default().entry(from.clone()).or_default().insert(url, now);
            if !state.first_sight(&message, now) {
                return;
            }
            if message.get("type").and_then(Value::as_str) != Some("presence") {
                drop(state);
                self.emit(SignalEvent::Message { space_id: space_id.to_owned(), message });
                return;
            }

            if state.retired.contains_key(&from) {
                return;
            }
            let was_online = !state.peers(space_id, now).is_empty();
            let peers = state.presence.entry(space_id.to_owned()).or_default();
            let known = peers.get(&from).is_some_and(|entry| now - entry.seen_at < PRESENCE_TTL_MS);
            let mut list_changed = false;
            let mut replaced = Vec::new();

            if message.get("online").is_some_and(is_truthy) {
                let info = describe_peer(&from, &message);
                list_changed = !known || peers.get(&from).is_some_and(|entry| entry.info != info);
                let name = info.name.clone();
                // A new instance of someone already here is that person's app restarted: the
                // old one is gone at once instead of lingering as a copy until it times out,
                // and a call moves to the new one right away.
                if let Some(member_id) = info.member_id.clone().filter(|_| !known) {
                    replaced = peers
                        .iter()
                        .filter(|(id, entry)| **id != from && entry.info.member_id.as_deref() == Some(member_id.as_str()))
                        .map(|(id, _)| id.clone())
                        .collect();
                    list_changed |= !replaced.is_empty();
                }
                peers.insert(from.clone(), PresenceEntry { seen_at: now, info });
                if !name.is_empty() {
                    events.push(SignalEvent::PeerName { space_id: space_id.to_owned(), name });
                }
                events.push(SignalEvent::PeerPresence {
                    space_id: space_id.to_owned(),
                    peer_id: from.clone(),
                    message: message.clone(),
                });
                answer = !known;
            } else if peers.shift_remove(&from).is_some() {
                list_changed = true;
            }

            // The old instance leaves every space it was in, not only this one.
            let mut other_spaces = Vec::new();
            for old in replaced {
                for (other, peers) in state.presence.iter_mut() {
                    if peers.shift_remove(&old).is_some() && other != space_id {
                        other_spaces.push(other.clone());
                    }
                }
                state.retired.insert(old, now);
                if state.retired.len() > MAX_SEEN {
                    state.retired.shift_remove_index(0);
                }
            }
            for other in other_spaces {
                let peers = state.peers(&other, now);
                events.push(SignalEvent::Peers { space_id: other, peers });
            }

            let peers = state.peers(space_id, now);
            let online = !peers.is_empty();
            if list_changed {
                events.push(SignalEvent::Peers { space_id: space_id.to_owned(), peers });
            }
            if was_online != online {
                events.push(SignalEvent::Online { space_id: space_id.to_owned(), online });
            }
        }
        for event in events {
            self.emit(event);
        }
        // Someone new hears about us right away instead of waiting for the next round.
        if answer {
            self.publish_presence(true, Some(space_id));
        }
    }

    /// Treats `message` as if it had just arrived from a message server, sealed with the key of
    /// the space. For tests and tools that need the app to see someone who is not there.
    /// From now on, everything this app sends goes to `sink` instead of the message servers, as
    /// the space and the message before it is sealed. For the tests, which wire two apps
    /// together with it and `loopback`.
    pub fn tap(&self, sink: mpsc::UnboundedSender<(String, Value)>) {
        self.lock().tap = Some(sink);
    }

    pub fn loopback(&self, space_id: &str, message: &Value) {
        let Some(key) = self.identity.key_for(space_id) else {
            return;
        };
        if self.lock().links.is_empty() {
            return;
        }
        self.handle_message(&format!("{TOPIC_PREFIX}{space_id}"), &seal_message(&key, message), 0);
    }

    pub fn peers(&self, space_id: &str) -> Vec<Peer> {
        self.lock().peers(space_id, now_ms())
    }

    pub fn is_online(&self, space_id: &str) -> bool {
        !self.peers(space_id).is_empty()
    }

    pub fn status(&self) -> Status {
        self.lock().status()
    }

    /// Drops who has not been heard for a while, and old message ids.
    fn expire(&self, now: i64) {
        let mut events = Vec::new();
        {
            let mut state = self.lock();
            state.expire_seen(now);
            let State { presence, routes, .. } = &mut *state;
            for (space_id, peers) in presence.iter_mut() {
                let before = peers.len();
                peers.retain(|_, entry| now - entry.seen_at < PRESENCE_TTL_MS);
                if let Some(routes) = routes.get_mut(space_id) {
                    routes.retain(|peer_id, _| peers.contains_key(peer_id));
                }
                if peers.len() != before {
                    let list: Vec<Peer> = peers.values().map(|entry| entry.info.clone()).collect();
                    let empty = list.is_empty();
                    events.push(SignalEvent::Peers { space_id: space_id.clone(), peers: list });
                    if empty {
                        events.push(SignalEvent::Online { space_id: space_id.clone(), online: false });
                    }
                }
            }
        }
        for event in events {
            self.emit(event);
        }
    }

    /// Says goodbye everywhere and disconnects. The goodbye is not confirmed, so this is quick.
    pub async fn stop(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        let now = now_ms();
        let goodbyes: Vec<(String, String)> = self
            .identity
            .space_ids()
            .into_iter()
            .filter_map(|id| {
                let key = self.identity.key_for(&id)?;
                let message = json!({
                    "type": "presence", "online": false, "from": self.instance_id, "at": now, "mid": new_uuid(),
                });
                Some((format!("{TOPIC_PREFIX}{id}"), seal_message(&key, &message)))
            })
            .collect();
        {
            let mut state = self.lock();
            for link in state.links.iter_mut().filter(|link| link.connected) {
                let Some(client) = link.client.as_ref() else { continue };
                for (topic, payload) in &goodbyes {
                    if client.try_publish(topic.clone(), QoS::AtMostOnce, false, payload.clone()).is_ok() {
                        link.queued.push_back(None);
                    }
                }
            }
        }
        // Gives the goodbyes time to be written before the link tasks disconnect.
        tokio::time::sleep(Duration::from_millis(250)).await;
        self.stop.notify_waiters();
        tokio::time::sleep(Duration::from_millis(350)).await;
    }
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        _ => true,
    }
}

fn to_base36(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut text = Vec::new();
    loop {
        text.push(DIGITS[(value % 36) as usize]);
        value /= 36;
        if value == 0 {
            break;
        }
    }
    text.reverse();
    String::from_utf8(text).expect("base 36 digits are ASCII")
}

fn mqtt_options(address: &str, instance_id: &str) -> Option<MqttOptions> {
    let url = url::Url::parse(address).ok()?;
    let secure = match url.scheme() {
        "wss" => true,
        "ws" => false,
        _ => return None,
    };
    let port = url.port().unwrap_or(if secure { 443 } else { 80 });
    // The client takes the whole address for a websocket, without the user and password in it.
    let query = url.query().map_or_else(String::new, |query| format!("?{query}"));
    let bare = format!("{}://{}:{port}{}{query}", url.scheme(), url.host_str()?, url.path());

    let client_id = format!("telinha-{}-{}", &instance_id[..8.min(instance_id.len())], to_base36(now_ms() as u64));
    let mut options = MqttOptions::new(client_id, bare, port);
    options.set_transport(if secure { Transport::wss_with_default_config() } else { Transport::Ws });
    options.set_keep_alive(Duration::from_secs(30));
    options.set_clean_session(true);
    options.set_max_packet_size(MAX_PACKET_BYTES, MAX_PACKET_BYTES);
    if !url.username().is_empty() {
        options.set_credentials(url.username(), url.password().unwrap_or(""));
    }
    Some(options)
}

/// How many whole numbers a value holds, for fields other apps fill in.
pub fn whole_number(value: Option<&Value>) -> i64 {
    value.map_or(0, number_of)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OneSpace {
        key: [u8; 32],
    }

    impl Identity for OneSpace {
        fn space_ids(&self) -> Vec<String> {
            vec!["space".into()]
        }

        fn key_for(&self, space_id: &str) -> Option<[u8; 32]> {
            (space_id == "space").then_some(self.key)
        }

        fn profile_name(&self) -> String {
            "Rafa".into()
        }

        fn presence_for(&self, _space_id: &str) -> Map<String, Value> {
            Map::new()
        }
    }

    fn signaling(brokers: &[&str]) -> (Arc<Signaling>, mpsc::UnboundedReceiver<SignalEvent>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let brokers = brokers.iter().map(|url| Broker::new(url)).collect();
        (Signaling::new(Arc::new(OneSpace { key: [3u8; 32] }), brokers, sender), receiver)
    }

    fn deliver(signaling: &Signaling, link: usize, message: Value) {
        let payload = seal_message(&[3u8; 32], &message);
        signaling.handle_message(&format!("{TOPIC_PREFIX}space"), &payload, link);
    }

    fn drain(receiver: &mut mpsc::UnboundedReceiver<SignalEvent>) -> Vec<SignalEvent> {
        std::iter::from_fn(|| receiver.try_recv().ok()).collect()
    }

    fn presence(from: &str, extra: Value) -> Value {
        let mut message = json!({ "type": "presence", "online": true, "from": from, "at": now_ms(), "mid": new_uuid() });
        message.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        message
    }

    #[test]
    fn a_restarted_app_replaces_the_old_copy_of_the_person() {
        let (signaling, mut events) = signaling(&["wss://a"]);
        let member = json!({ "name": "Rafa", "memberId": "c".repeat(32) });
        deliver(&signaling, 0, presence("peer-old", member.clone()));
        deliver(&signaling, 0, presence("peer-other", json!({ "name": "Keep", "memberId": "b".repeat(32) })));
        drain(&mut events);
        deliver(&signaling, 0, presence("peer-new", member.clone()));
        let ids = |signaling: &Signaling| {
            let mut ids: Vec<String> = signaling.peers("space").into_iter().map(|peer| peer.id).collect();
            ids.sort();
            ids
        };
        assert_eq!(ids(&signaling), ["peer-new", "peer-other"]);
        assert!(drain(&mut events).iter().any(|event| matches!(event, SignalEvent::Peers { .. })));
        // A late message of the old app does not bring the copy back.
        deliver(&signaling, 0, presence("peer-old", member));
        assert_eq!(ids(&signaling), ["peer-new", "peer-other"]);
    }

    #[test]
    fn someone_announcing_themselves_shows_up_once_and_leaves_on_goodbye() {
        let (signaling, mut events) = signaling(&["wss://a"]);
        deliver(&signaling, 0, presence("peer-1", json!({ "name": "Keep", "memberId": "b".repeat(32) })));
        let first = drain(&mut events);
        assert!(first.contains(&SignalEvent::PeerName { space_id: "space".into(), name: "Keep".into() }));
        assert!(first.contains(&SignalEvent::Online { space_id: "space".into(), online: true }));
        assert_eq!(signaling.peers("space")[0].member_id.as_deref(), Some("b".repeat(32).as_str()));

        // The same announcement again changes nothing in the list.
        deliver(&signaling, 0, presence("peer-1", json!({ "name": "Keep", "memberId": "b".repeat(32) })));
        assert!(!drain(&mut events).iter().any(|event| matches!(event, SignalEvent::Peers { .. })));

        deliver(&signaling, 0, json!({ "type": "presence", "online": false, "from": "peer-1", "at": now_ms(), "mid": "bye" }));
        let last = drain(&mut events);
        assert!(last.contains(&SignalEvent::Peers { space_id: "space".into(), peers: vec![] }));
        assert!(last.contains(&SignalEvent::Online { space_id: "space".into(), online: false }));
        assert!(!signaling.is_online("space"));
    }

    #[test]
    fn the_copy_that_arrives_through_another_server_is_dropped() {
        let (signaling, mut events) = signaling(&["wss://a", "wss://b"]);
        let chat = json!({ "type": "chat", "from": "peer-1", "at": now_ms(), "mid": "one-id", "text": "oi" });
        deliver(&signaling, 0, chat.clone());
        deliver(&signaling, 1, chat);
        let received = drain(&mut events);
        assert_eq!(received.len(), 1);
        assert!(matches!(&received[0], SignalEvent::Message { message, .. } if message["text"] == "oi"));

        // An old app sends no id and is never taken for a copy.
        let old = json!({ "type": "chat", "from": "peer-1", "at": now_ms(), "text": "antiga" });
        deliver(&signaling, 0, old.clone());
        deliver(&signaling, 0, old);
        assert_eq!(drain(&mut events).len(), 2);
    }

    #[test]
    fn our_own_echo_and_messages_for_another_key_are_ignored() {
        let (signaling, mut events) = signaling(&["wss://a"]);
        let echo = json!({ "type": "chat", "from": signaling.instance_id(), "at": now_ms(), "mid": "x" });
        deliver(&signaling, 0, echo);
        let foreign = seal_message(&[9u8; 32], &json!({ "type": "chat", "from": "p", "at": now_ms() }));
        signaling.handle_message(&format!("{TOPIC_PREFIX}space"), &foreign, 0);
        signaling.handle_message("other/topic", &foreign, 0);
        assert!(drain(&mut events).is_empty());
    }

    #[test]
    fn a_peer_is_described_only_by_what_fits_the_format() {
        let message = json!({
            "name": "x".repeat(60),
            "memberId": "not an id",
            "voice": { "roomId": "dm", "mic": false, "camera": true, "live": true, "liveName": "Jogo", "v": 2 },
            "style": { "bio": "oi" },
        });
        let peer = describe_peer("peer-1", &message);
        assert_eq!(peer.name.len(), 40);
        assert_eq!(peer.member_id, None);
        let voice = peer.voice.unwrap();
        assert_eq!((voice.room_id.as_str(), voice.mic, voice.camera, voice.deaf, voice.live, voice.version), ("dm", false, true, false, true, 2));
        assert_eq!(peer.style.unwrap().bio, "oi");

        let plain = describe_peer("peer-2", &json!({ "name": "a", "voice": { "roomId": "bad room!" } }));
        assert!(plain.voice.is_none() && plain.style.is_none());
        let default_voice = describe_peer("peer-3", &json!({ "voice": { "roomId": "abc" } })).voice.unwrap();
        assert!(default_voice.mic && default_voice.version == 1);
    }

    #[test]
    fn who_is_not_heard_for_a_while_goes_offline() {
        let (signaling, mut events) = signaling(&["wss://a"]);
        deliver(&signaling, 0, presence("peer-1", json!({ "name": "Keep" })));
        drain(&mut events);
        signaling.expire(now_ms() + PRESENCE_TTL_MS - 1000);
        assert!(drain(&mut events).is_empty());
        signaling.expire(now_ms() + PRESENCE_TTL_MS + 1000);
        let events = drain(&mut events);
        assert!(events.contains(&SignalEvent::Online { space_id: "space".into(), online: false }));
    }

    #[test]
    fn a_large_message_goes_only_where_the_people_are() {
        let (signaling, _events) = signaling(&["wss://a", "wss://b", "wss://c"]);
        deliver(&signaling, 1, presence("peer-1", json!({ "name": "Keep" })));
        deliver(&signaling, 2, presence("peer-2", json!({ "name": "nanda" })));
        deliver(&signaling, 1, presence("peer-2", json!({ "name": "nanda" })));
        let state = signaling.lock();
        let now = now_ms();
        assert_eq!(state.route_for("space", 100, vec![0, 1, 2], now), [0, 1, 2]);
        assert_eq!(state.route_for("space", WIDE_PAYLOAD_BYTES + 1, vec![0, 1, 2], now), [1]);
        assert_eq!(state.route_for("space", WIDE_PAYLOAD_BYTES + 1, vec![0, 2], now), [0, 2]);
        assert_eq!(state.route_for("empty", WIDE_PAYLOAD_BYTES + 1, vec![0, 1, 2], now), [0]);
    }

    #[test]
    fn an_address_becomes_client_options_without_the_password_in_it() {
        let options = mqtt_options("wss://public:public@public.cloud.shiftr.io:443", "0123456789abcdef").unwrap();
        assert_eq!(options.broker_address(), ("wss://public.cloud.shiftr.io:443/".to_owned(), 443));
        assert!(options.credentials().is_some());
        assert!(options.client_id().starts_with("telinha-01234567-"));
        let plain = mqtt_options("wss://broker.hivemq.com:8884/mqtt", "0123456789abcdef").unwrap();
        assert_eq!(plain.broker_address(), ("wss://broker.hivemq.com:8884/mqtt".to_owned(), 8884));
        assert!(mqtt_options("mqtt://nope", "0123456789abcdef").is_none());
        assert_eq!(to_base36(35), "z");
        assert_eq!(to_base36(36), "10");
    }
}
