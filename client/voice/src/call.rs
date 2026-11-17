//! A voice call: one WebRTC connection to each person in the room. The negotiation messages are
//! the ones `desktop/renderer/call.js` exchanges, so a call mixes people on both apps.
//!
//! Between two people, the one whose app id sorts first makes the offer. Every offer opens a new
//! session, and messages of another session are ignored.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use libwebrtc::MediaType;
use libwebrtc::audio_frame::AudioFrame;
use libwebrtc::audio_source::native::NativeAudioSource;
use libwebrtc::audio_source::AudioSourceOptions;
use libwebrtc::audio_stream::native::NativeAudioStream;
use libwebrtc::audio_track::RtcAudioTrack;
use libwebrtc::ice_candidate::IceCandidate;
use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::peer_connection::{AnswerOptions, OfferOptions, PeerConnection, PeerConnectionState, SignalingState};
use libwebrtc::peer_connection_factory::native::PeerConnectionFactoryExt;
use libwebrtc::peer_connection_factory::{IceServer, PeerConnectionFactory, RtcConfiguration};
use libwebrtc::rtp_transceiver::{RtpTransceiverDirection, RtpTransceiverInit};
use libwebrtc::session_description::{SdpType, SessionDescription};
use libwebrtc::video_source::VideoResolution;
use libwebrtc::video_source::native::NativeVideoSource;
use libwebrtc::video_track::RtcVideoTrack;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::audio::{AudioIo, FRAME, Level, Mixer, Processing, RATE, level_of};
use crate::video::{FrameStore, Waker, watch_track};

/// The videos that arrive, one store for each person, shared with the window.
type Videos = Arc<std::sync::Mutex<HashMap<String, Arc<FrameStore>>>>;

const SPEAKING_LEVEL: f32 = 0.03;
const SPEAKING_HOLD: Duration = Duration::from_millis(350);
const RETRY_DELAY: Duration = Duration::from_secs(3);
const DROP_RETRY_DELAY: Duration = Duration::from_secs(1);
const DROP_GRACE: Duration = Duration::from_secs(2);
const QUICK_ANSWER: Duration = Duration::from_secs(5);
const OFFER_REPEAT_AT: [Duration; 4] =
    [Duration::from_secs(4), Duration::from_secs(10), Duration::from_secs(22), Duration::from_secs(46)];
const ANSWER_TIMEOUT: Duration = Duration::from_secs(90);
const PEER_GRACE: Duration = Duration::from_secs(5);
const MAX_PENDING_CANDIDATES: usize = 100;
/// How often the connections are asked for their numbers, as `STATS_INTERVAL_MS` in call.js.
const STATS_INTERVAL: Duration = Duration::from_secs(2);

pub fn default_ice_servers() -> Vec<IceServer> {
    vec![IceServer {
        urls: vec!["stun:stun.l.google.com:19302".into(), "stun:stun.cloudflare.com:3478".into()],
        username: String::new(),
        password: String::new(),
    }]
}

/// Where the sound of a call comes from and goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    /// The microphone and the speakers of the computer.
    Devices,
    /// A steady tone in place of the microphone and nothing played, for tests: two calls on
    /// one computer with real devices would hear each other and howl.
    Tone,
}

/// Feeds a 440 Hz tone as if it were the microphone.
async fn tone(captured: mpsc::UnboundedSender<Vec<i16>>, mic_on: Arc<AtomicBool>) {
    let mut clock = tokio::time::interval(Duration::from_millis(10));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut phase = 0.0f32;
    loop {
        clock.tick().await;
        let on = mic_on.load(Ordering::Relaxed);
        let frame: Vec<i16> = (0..FRAME)
            .map(|_| {
                phase += std::f32::consts::TAU * 440.0 / RATE as f32;
                if on { (phase.sin() * 12000.0) as i16 } else { 0 }
            })
            .collect();
        if captured.send(frame).is_err() {
            return;
        }
    }
}

/// Someone in the room, as the presence describes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    pub id: String,
    /// Their app answers again when it gets the same offer twice, instead of starting over.
    pub answers_repeats: bool,
    /// How to name them in the log.
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    Connecting,
    Connected,
    /// The connection dropped or never closed. A new attempt is on the way.
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallEvent {
    Peer { id: String, state: PeerState },
    /// Someone started or stopped speaking. The id of this app stands for the microphone.
    Speaking { id: String, speaking: bool },
    /// A line for the app's log: how long a connection took, or that it failed.
    Report(String),
    /// Something the person has to know: the microphone or the speakers did not open.
    Problem(String),
    /// The numbers of the call, every two seconds while it runs.
    Stats(CallStats),
    /// The answer to the offer of a screen to watch here, to send back to who shares it.
    StreamAnswer { sharer_id: String, sdp: String },
    /// A screen watched here connected, or failed.
    StreamState { sharer_id: String, live: bool },
    /// The numbers of a screen watched here, every two seconds while it plays.
    StreamStats { sharer_id: String, stats: StreamStats },
}

/// What the connection of a screen watched here measured, as `collectStats` in streamview.js.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamStats {
    pub fps: Option<f64>,
    /// The size of the picture as it arrives.
    pub size: Option<(u32, u32)>,
    /// The round trip to who shares, in milliseconds.
    pub rtt: Option<f64>,
    /// The share of packets lost since the last reading, from 0 to 1.
    pub loss: Option<f64>,
    /// How fast the screen arrives, in kilobits per second.
    pub bitrate: Option<f64>,
    /// The delay of the buffer of the picture, in milliseconds.
    pub buffer: Option<f64>,
    pub dropped: u32,
    pub decoder: String,
}

/// The running counts of the picture of a screen, to tell what changed since the last reading.
#[derive(Debug, Clone, Copy)]
struct VideoCounts {
    lost: i64,
    received: u64,
    bytes: u64,
    delay: f64,
    emitted: u64,
    at: Instant,
}

/// What the connections of the call measured, as `collectStats` in call.js summed it up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallStats {
    /// People the call is trying to reach.
    pub peers: usize,
    /// How many of them are connected.
    pub connected: usize,
    /// The average round trip to the connected people, in milliseconds.
    pub rtt: Option<f64>,
    /// The worst share of packets lost since the last reading, from 0 to 1.
    pub loss: Option<f64>,
    /// The worst share of voice the receiver had to invent, from 0 to 1.
    pub concealment: Option<f64>,
    /// The worst share of voice stretched or squeezed to keep the delay.
    pub stretch: Option<f64>,
    /// The worst delay of the voice buffer, in milliseconds.
    pub buffer: Option<f64>,
}

/// The running counts of one connection, to tell what changed since the last reading.
#[derive(Debug, Clone, Copy, Default)]
struct Counts {
    lost: i64,
    received: u64,
    audio: Option<AudioCounts>,
}

#[derive(Debug, Clone, Copy, Default)]
struct AudioCounts {
    samples: u64,
    concealed: u64,
    stretched: u64,
    delay: f64,
    emitted: u64,
}

/// One connection's reading.
struct Reading {
    rtt: Option<f64>,
    loss: f64,
    concealment: Option<f64>,
    stretch: Option<f64>,
    buffer: Option<f64>,
}

/// The round trip of the chosen pair of addresses of a connection, in milliseconds.
fn round_trip(stats: &[libwebrtc::stats::RtcStats]) -> Option<f64> {
    use libwebrtc::stats::RtcStats;
    let selected = stats.iter().find_map(|stat| match stat {
        RtcStats::Transport(transport) if !transport.transport.selected_candidate_pair_id.is_empty() => Some(transport.transport.selected_candidate_pair_id.as_str()),
        _ => None,
    });
    let pairs = stats.iter().filter_map(|stat| match stat {
        RtcStats::CandidatePair(pair) => Some(pair),
        _ => None,
    });
    let mut pair = None;
    for candidate in pairs {
        if selected == Some(candidate.rtc.id.as_str()) {
            pair = Some(candidate);
            break;
        }
        if pair.is_none() && candidate.candidate_pair.nominated && candidate.candidate_pair.state == Some(libwebrtc::stats::IceCandidatePairState::Succeeded) {
            pair = Some(candidate);
        }
    }
    // A round trip under a millisecond reads as zero; it counts once a check came back.
    pair.filter(|pair| pair.candidate_pair.responses_received > 0)
        .map(|pair| pair.candidate_pair.current_round_trip_time)
        .filter(|rtt| rtt.is_finite() && *rtt >= 0.0)
        .map(|rtt| rtt * 1000.0)
}

/// Reads the report of a screen watched here: the picture that arrived and the round trip.
fn read_stream_stats(stats: &[libwebrtc::stats::RtcStats], before: Option<VideoCounts>, now: Instant) -> Option<(StreamStats, VideoCounts)> {
    use libwebrtc::stats::RtcStats;
    let video = stats.iter().find_map(|stat| match stat {
        RtcStats::InboundRtp(inbound) if inbound.stream.kind == "video" => Some(inbound),
        _ => None,
    })?;
    let item = &video.inbound;
    let counts = VideoCounts {
        lost: video.received.packets_lost,
        received: video.received.packets_received,
        bytes: item.bytes_received,
        delay: item.jitter_buffer_delay,
        emitted: item.jitter_buffer_emitted_count,
        at: now,
    };
    let (loss, bitrate, buffer) = match before {
        Some(then) => {
            let lost = (counts.lost - then.lost).max(0) as f64;
            let received = counts.received.saturating_sub(then.received) as f64;
            let seconds = now.duration_since(then.at).as_secs_f64();
            let bitrate = (seconds > 0.0).then(|| counts.bytes.saturating_sub(then.bytes) as f64 * 8.0 / seconds / 1000.0);
            let buffer = (counts.emitted > then.emitted).then(|| (counts.delay - then.delay) / (counts.emitted - then.emitted) as f64 * 1000.0);
            ((lost + received > 0.0).then(|| lost / (lost + received)), bitrate, buffer)
        }
        None => (None, None, None),
    };
    let reading = StreamStats {
        fps: (item.frames_per_second > 0.0).then_some(item.frames_per_second),
        size: (item.frame_width > 0 && item.frame_height > 0).then_some((item.frame_width, item.frame_height)),
        rtt: round_trip(stats),
        loss,
        bitrate,
        buffer,
        dropped: item.frames_dropped,
        decoder: item.decoder_implementation.clone(),
    };
    Some((reading, counts))
}

/// Reads one connection's report the way call.js does: the round trip of the chosen pair of
/// addresses, and the packets and the voice that arrived.
fn read_stats(stats: &[libwebrtc::stats::RtcStats], before: Counts) -> (Reading, Counts) {
    use libwebrtc::stats::RtcStats;
    let mut lost = 0i64;
    let mut received = 0u64;
    let mut audio = None;
    for stat in stats {
        match stat {
            RtcStats::InboundRtp(inbound) => {
                lost += inbound.received.packets_lost;
                received += inbound.received.packets_received;
                if inbound.stream.kind == "audio" {
                    let item = &inbound.inbound;
                    audio = Some(AudioCounts {
                        samples: item.total_samples_received,
                        concealed: item.concealed_samples,
                        stretched: item.inserted_samples_for_deceleration + item.removed_samples_for_acceleration,
                        delay: item.jitter_buffer_delay,
                        emitted: item.jitter_buffer_emitted_count,
                    });
                }
            }
            _ => {}
        }
    }
    let rtt = round_trip(stats);
    let delta_lost = (lost - before.lost).max(0) as f64;
    let delta_received = received.saturating_sub(before.received) as f64;
    let loss = if delta_lost + delta_received > 0.0 { delta_lost / (delta_lost + delta_received) } else { 0.0 };
    let (concealment, stretch, buffer) = match (audio, before.audio) {
        (Some(now), Some(then)) => {
            let samples = now.samples.saturating_sub(then.samples) as f64;
            let share = |after: u64, earlier: u64| (samples > 0.0).then(|| after.saturating_sub(earlier) as f64 / samples);
            let buffer = (now.emitted > then.emitted).then(|| (now.delay - then.delay) / (now.emitted - then.emitted) as f64 * 1000.0);
            (share(now.concealed, then.concealed), share(now.stretched, then.stretched), buffer)
        }
        _ => (None, None, None),
    };
    (Reading { rtt, loss, concealment, stretch, buffer }, Counts { lost, received, audio })
}

enum Command {
    Participants(Vec<Participant>),
    Rtc { from: String, payload: Value },
    Candidate { id: String, serial: u64, candidate: IceCandidate },
    State { id: String, serial: u64, state: PeerConnectionState },
    Track { id: String, serial: u64, track: RtcAudioTrack },
    Video { id: String, serial: u64, track: RtcVideoTrack },
    /// A wait is over. `step` says which repeat of the offer it is, when it is one.
    Timer { id: String, serial: u64, kind: TimerKind },
    Voice { id: String, level: f32 },
    /// A screen to watch inside the window, offered by the engine of who shares it.
    StreamOffer { sharer: String, sdp: String },
    StreamClose { sharer: String },
    StreamState { sharer: String, serial: u64, state: PeerConnectionState },
    StreamTrack { sharer: String, serial: u64, track: MediaStreamTrack },
    Stop,
}

/// A screen being watched inside the window: one connection that only receives.
struct StreamView {
    connection: PeerConnection,
    serial: u64,
    live: bool,
    counts: Option<VideoCounts>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

/// The id of the screen of this app in its own preview, as `SELF_VIEWER` in streams.js.
pub const SELF_STREAM: &str = "self";

/// The key of the sound of a screen in the mixer, apart from the voice of its sharer.
fn stream_key(sharer: &str) -> String {
    format!("stream:{sharer}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerKind {
    AwaitAnswer(usize),
    Retry,
    Drop,
}

struct Peer {
    initiator: bool,
    session: String,
    connection: Option<PeerConnection>,
    /// Changes with every new connection, so what an old one still reports is ignored.
    serial: u64,
    pending: Vec<(String, Value)>,
    state: PeerState,
    created: Instant,
    started: Instant,
    offered: Instant,
    answer_time: Option<Duration>,
    attempt: u32,
    connected: bool,
    speaking_until: Option<Instant>,
    local_answer: Option<Value>,
    counts: Counts,
}

/// A handle to a call in progress. Dropping it does not end the call; `stop` does.
#[derive(Clone)]
pub struct Call {
    commands: mpsc::UnboundedSender<Command>,
    mixer: Arc<Mixer>,
    mic_on: Arc<AtomicBool>,
    /// What the camera of this app sends. With no frames fed, nothing goes out.
    camera: NativeVideoSource,
    videos: Videos,
    /// The screens watched inside the window, by who shares them.
    stream_videos: Videos,
    waker: Arc<std::sync::Mutex<Option<Waker>>>,
}

impl Call {
    /// Enters a call. `send` delivers a negotiation message to one person. Must be called from
    /// inside a tokio runtime.
    pub fn start(
        self_id: &str,
        ice_servers: Vec<IceServer>,
        sound: Sound,
        mic_on: bool,
        deaf: bool,
        send: impl Fn(&str, Value) + Send + Sync + 'static,
    ) -> (Self, mpsc::UnboundedReceiver<CallEvent>) {
        let (commands, inbox) = mpsc::unbounded_channel();
        let (events, receiver) = mpsc::unbounded_channel();
        let mixer = Arc::new(Mixer::default());
        mixer.set_deaf(deaf);
        let mic = Arc::new(AtomicBool::new(mic_on));
        let camera = NativeVideoSource::new(VideoResolution { width: 1280, height: 720 }, false);
        let videos: Videos = Arc::default();
        let stream_videos: Videos = Arc::default();
        let waker: Arc<std::sync::Mutex<Option<Waker>>> = Arc::default();
        let actor = Actor {
            stream_videos: Arc::clone(&stream_videos),
            streams: HashMap::new(),
            camera: camera.clone(),
            videos: Arc::clone(&videos),
            waker: Arc::clone(&waker),
            video_track: None,
            watchers: HashMap::new(),
            self_id: self_id.to_owned(),
            ice_servers,
            sound,
            send: Box::new(send),
            events,
            commands: commands.clone(),
            mixer: Arc::clone(&mixer),
            mic_on: Arc::clone(&mic),
            peers: HashMap::new(),
            participants: Vec::new(),
            serial: 0,
        };
        tokio::spawn(actor.run(inbox));
        (Self { commands, mixer, mic_on: mic, camera, videos, stream_videos, waker }, receiver)
    }

    pub fn set_participants(&self, participants: Vec<Participant>) {
        let _ = self.commands.send(Command::Participants(participants));
    }

    /// A negotiation message that arrived from someone in the call.
    pub fn handle_rtc(&self, from: &str, payload: Value) {
        let _ = self.commands.send(Command::Rtc { from: from.to_owned(), payload });
    }

    pub fn set_mic(&self, on: bool) {
        self.mic_on.store(on, Ordering::Relaxed);
    }

    pub fn set_deaf(&self, deaf: bool) {
        self.mixer.set_deaf(deaf);
    }

    /// The volume of one person, in percent of what arrives, up to 200.
    pub fn set_volume(&self, id: &str, percent: f32) {
        self.mixer.set_volume(id, percent / 100.0);
    }

    /// Fires a sound of the soundboard into the call: decoded, 48 kHz mono.
    pub fn play_sound(&self, samples: Arc<[i16]>, volume: f32) {
        self.mixer.play_sound(samples, volume);
    }

    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
    }

    /// Where the frames of the camera of this app go to reach everyone in the call.
    pub fn camera(&self) -> NativeVideoSource {
        self.camera.clone()
    }

    /// The video that arrives from someone, when it has started.
    pub fn video_of(&self, id: &str) -> Option<Arc<FrameStore>> {
        self.videos.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(id).cloned()
    }

    /// Opens a screen inside the window from the offer of its engine. The answer comes back as
    /// `CallEvent::StreamAnswer`.
    pub fn open_stream(&self, sharer_id: &str, sdp: &str) {
        let _ = self.commands.send(Command::StreamOffer { sharer: sharer_id.to_owned(), sdp: sdp.to_owned() });
    }

    pub fn close_stream(&self, sharer_id: &str) {
        let _ = self.commands.send(Command::StreamClose { sharer: sharer_id.to_owned() });
    }

    /// The picture of a screen watched inside the window, once it started.
    pub fn stream_video(&self, sharer_id: &str) -> Option<Arc<FrameStore>> {
        self.stream_videos.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(sharer_id).cloned()
    }

    /// How loud the sound of a screen plays, in percent, up to 200.
    pub fn set_stream_volume(&self, sharer_id: &str, percent: f32) {
        self.mixer.set_volume(&stream_key(sharer_id), percent / 100.0);
    }

    /// What to call when a new frame arrives, to draw it.
    pub fn set_waker(&self, waker: Waker) {
        *self.waker.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(waker);
    }
}

struct Actor {
    self_id: String,
    ice_servers: Vec<IceServer>,
    sound: Sound,
    send: Box<dyn Fn(&str, Value) + Send + Sync>,
    events: mpsc::UnboundedSender<CallEvent>,
    commands: mpsc::UnboundedSender<Command>,
    mixer: Arc<Mixer>,
    mic_on: Arc<AtomicBool>,
    peers: HashMap<String, Peer>,
    participants: Vec<Participant>,
    serial: u64,
    camera: NativeVideoSource,
    video_track: Option<RtcVideoTrack>,
    videos: Videos,
    waker: Arc<std::sync::Mutex<Option<Waker>>>,
    /// The task reading each person's video.
    watchers: HashMap<String, tokio::task::JoinHandle<()>>,
    streams: HashMap<String, StreamView>,
    stream_videos: Videos,
}

impl Actor {
    async fn run(mut self, mut inbox: mpsc::UnboundedReceiver<Command>) {
        let factory = crate::factory();
        let source = NativeAudioSource::new(AudioSourceOptions::default(), RATE, 1, 100);
        let track = factory.create_audio_track("telinha-mic", source.clone());
        self.video_track = Some(factory.create_video_track("telinha-camera", self.camera.clone()));

        let processing = Arc::new(Processing::new());
        let level = Arc::new(Level::default());
        let (captured, mut frames) = mpsc::unbounded_channel::<Vec<i16>>();
        let (problems, mut problem_lines) = mpsc::unbounded_channel::<String>();
        let audio = match self.sound {
            Sound::Devices => Some(AudioIo::start(processing, Arc::clone(&self.mixer), Arc::clone(&self.mic_on), Arc::clone(&level), captured, problems)),
            Sound::Tone => {
                tokio::spawn(tone(captured, Arc::clone(&self.mic_on)));
                None
            }
        };

        // The microphone goes to the connections from its own task, so a slow negotiation never
        // holds the voice back.
        let feeder = tokio::spawn(async move {
            while let Some(samples) = frames.recv().await {
                let frame = AudioFrame { data: samples.into(), sample_rate: RATE, num_channels: 1, samples_per_channel: FRAME as u32 };
                let _ = source.capture_frame(&frame).await;
            }
        });

        let mut meter = tokio::time::interval(Duration::from_millis(100));
        let mut stats = tokio::time::interval(STATS_INTERVAL);
        stats.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut mic_speaking = false;
        let mut mic_until: Option<Instant> = None;
        loop {
            tokio::select! {
                command = inbox.recv() => match command {
                    None | Some(Command::Stop) => break,
                    Some(command) => self.handle(&factory, &track, command).await,
                },
                Some(problem) = problem_lines.recv() => self.emit(CallEvent::Problem(problem)),
                _ = meter.tick() => {
                    let now = Instant::now();
                    if level.get() > SPEAKING_LEVEL {
                        mic_until = Some(now + SPEAKING_HOLD);
                    }
                    let speaking = mic_until.is_some_and(|until| now < until);
                    if speaking != mic_speaking {
                        mic_speaking = speaking;
                        self.emit(CallEvent::Speaking { id: self.self_id.clone(), speaking });
                    }
                    self.expire_speaking(now);
                }
                _ = stats.tick() => self.collect_stats().await,
            }
        }

        for (id, peer) in self.peers.drain() {
            if let Some(connection) = peer.connection {
                connection.close();
            }
            self.mixer.remove(&id);
        }
        for (_, watcher) in self.watchers.drain() {
            watcher.abort();
        }
        self.videos.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
        let sharers: Vec<String> = self.streams.keys().cloned().collect();
        for sharer in sharers {
            self.close_stream(&sharer);
        }
        self.mixer.clear();
        feeder.abort();
        drop(audio);
    }

    /// Reads every connected connection and sends the summary, as `collectStats` in call.js.
    async fn collect_stats(&mut self) {
        let connected: Vec<(String, u64, PeerConnection)> = self
            .peers
            .iter()
            .filter(|(_, peer)| peer.state == PeerState::Connected)
            .filter_map(|(id, peer)| peer.connection.clone().map(|connection| (id.clone(), peer.serial, connection)))
            .collect();
        let mut readings = Vec::new();
        for (id, serial, connection) in connected {
            let Ok(stats) = connection.get_stats().await else { continue };
            let Some(peer) = self.peers.get_mut(&id).filter(|peer| peer.serial == serial) else { continue };
            let (reading, counts) = read_stats(&stats, peer.counts);
            peer.counts = counts;
            readings.push(reading);
        }
        let rtts: Vec<f64> = readings.iter().filter_map(|reading| reading.rtt).collect();
        let worst = |values: Vec<f64>| values.into_iter().reduce(f64::max);
        let summary = CallStats {
            peers: self.peers.len(),
            connected: readings.len(),
            rtt: (!rtts.is_empty()).then(|| rtts.iter().sum::<f64>() / rtts.len() as f64),
            loss: worst(readings.iter().map(|reading| reading.loss).collect()),
            concealment: worst(readings.iter().filter_map(|reading| reading.concealment).collect()),
            stretch: worst(readings.iter().filter_map(|reading| reading.stretch).collect()),
            buffer: worst(readings.iter().filter_map(|reading| reading.buffer).collect()),
        };
        self.emit(CallEvent::Stats(summary));
        self.collect_stream_stats().await;
    }

    /// Reads every screen watched here, apart from the own one, as `collectStats` in streamview.js.
    async fn collect_stream_stats(&mut self) {
        let live: Vec<(String, u64, PeerConnection)> = self
            .streams
            .iter()
            .filter(|(sharer, view)| view.live && sharer.as_str() != SELF_STREAM)
            .map(|(sharer, view)| (sharer.clone(), view.serial, view.connection.clone()))
            .collect();
        for (sharer, serial, connection) in live {
            let Ok(stats) = connection.get_stats().await else { continue };
            let Some(view) = self.streams.get_mut(&sharer).filter(|view| view.serial == serial) else { continue };
            let Some((reading, counts)) = read_stream_stats(&stats, view.counts, Instant::now()) else { continue };
            view.counts = Some(counts);
            self.emit(CallEvent::StreamStats { sharer_id: sharer, stats: reading });
        }
    }

    fn emit(&self, event: CallEvent) {
        let _ = self.events.send(event);
    }

    fn name_of(&self, id: &str) -> String {
        let name = self.participants.iter().find(|participant| participant.id == id).map_or("?", |participant| participant.name.as_str());
        format!("{name} ({})", &id[..id.len().min(4)])
    }

    fn timer(&self, id: &str, serial: u64, kind: TimerKind, delay: Duration) {
        let commands = self.commands.clone();
        let id = id.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = commands.send(Command::Timer { id, serial, kind });
        });
    }

    fn expire_speaking(&mut self, now: Instant) {
        let quiet: Vec<String> = self
            .peers
            .iter_mut()
            .filter(|(_, peer)| peer.speaking_until.is_some_and(|until| now >= until))
            .map(|(id, peer)| {
                peer.speaking_until = None;
                id.clone()
            })
            .collect();
        for id in quiet {
            self.emit(CallEvent::Speaking { id, speaking: false });
        }
    }

    async fn handle(&mut self, factory: &PeerConnectionFactory, track: &RtcAudioTrack, command: Command) {
        match command {
            Command::Participants(participants) => {
                self.participants = participants;
                self.reconcile(factory, track).await;
            }
            Command::Rtc { from, payload } => self.rtc(factory, track, &from, payload).await,
            Command::Candidate { id, serial, candidate } => {
                if let Some(peer) = self.peers.get(&id).filter(|peer| peer.serial == serial) {
                    let payload = json!({
                        "kind": "candidate",
                        "session": peer.session,
                        "candidate": {
                            "candidate": candidate.candidate(),
                            "sdpMid": candidate.sdp_mid(),
                            "sdpMLineIndex": candidate.sdp_mline_index(),
                        },
                    });
                    (self.send)(&id, payload);
                }
            }
            Command::State { id, serial, state } => self.state_changed(&id, serial, state),
            Command::Track { id, serial, track } => {
                if self.peers.get(&id).is_some_and(|peer| peer.serial == serial) {
                    self.play(&id, serial, track);
                }
            }
            Command::Video { id, serial, track } => {
                if self.peers.get(&id).is_some_and(|peer| peer.serial == serial) {
                    self.watch_video(&id, track);
                }
            }
            Command::Timer { id, serial, kind } => self.timer_fired(factory, track, &id, serial, kind).await,
            Command::Voice { id, level } => {
                let Some(peer) = self.peers.get_mut(&id) else { return };
                if level > SPEAKING_LEVEL {
                    let was = peer.speaking_until.is_some();
                    peer.speaking_until = Some(Instant::now() + SPEAKING_HOLD);
                    if !was {
                        self.emit(CallEvent::Speaking { id, speaking: true });
                    }
                }
            }
            Command::StreamOffer { sharer, sdp } => self.open_stream(factory, sharer, sdp),
            Command::StreamClose { sharer } => self.close_stream(&sharer),
            Command::StreamState { sharer, serial, state } => {
                let Some(view) = self.streams.get_mut(&sharer).filter(|view| view.serial == serial) else { return };
                match state {
                    PeerConnectionState::Connected if !view.live => {
                        view.live = true;
                        self.emit(CallEvent::StreamState { sharer_id: sharer, live: true });
                    }
                    PeerConnectionState::Failed => {
                        self.close_stream(&sharer);
                        self.emit(CallEvent::StreamState { sharer_id: sharer, live: false });
                    }
                    _ => {}
                }
            }
            Command::StreamTrack { sharer, serial, track } => self.stream_track(sharer, serial, track),
            Command::Stop => {}
        }
    }

    /// Answers the offer of a screen, as `handleOffer` in streamview.js: one connection that
    /// only receives, with every candidate written into the answer.
    fn open_stream(&mut self, factory: &PeerConnectionFactory, sharer: String, sdp: String) {
        self.close_stream(&sharer);
        self.serial += 1;
        let serial = self.serial;
        let mut config = RtcConfiguration::default();
        config.ice_servers = self.ice_servers.clone();
        let Ok(connection) = factory.create_peer_connection(config) else {
            self.emit(CallEvent::StreamState { sharer_id: sharer, live: false });
            return;
        };
        let (commands, id) = (self.commands.clone(), sharer.clone());
        connection.on_track(Some(Box::new(move |event| {
            let _ = commands.send(Command::StreamTrack { sharer: id.clone(), serial, track: event.track });
        })));
        let (commands, id) = (self.commands.clone(), sharer.clone());
        connection.on_connection_state_change(Some(Box::new(move |state| {
            let _ = commands.send(Command::StreamState { sharer: id.clone(), serial, state });
        })));
        let gathering = crate::filelink::Gathering::watch(&connection);
        let (events, negotiating, id) = (self.events.clone(), connection.clone(), sharer.clone());
        let answer = tokio::spawn(async move {
            let answered = async {
                let offer = SessionDescription::parse(&richer_opus(&sdp), SdpType::Offer).ok()?;
                negotiating.set_remote_description(offer).await.ok()?;
                let answer = negotiating.create_answer(AnswerOptions::default()).await.ok()?;
                let text = answer.to_string();
                negotiating.set_local_description(answer).await.ok()?;
                Some(gathering.finish(text).await)
            };
            let event = match answered.await {
                Some(sdp) => CallEvent::StreamAnswer { sharer_id: id, sdp },
                None => CallEvent::StreamState { sharer_id: id, live: false },
            };
            let _ = events.send(event);
        });
        self.streams.insert(sharer, StreamView { connection, serial, live: false, counts: None, tasks: vec![answer] });
    }

    /// The picture of a screen goes to the window; its sound to the speakers, except for the
    /// screen of this app, which would only echo.
    fn stream_track(&mut self, sharer: String, serial: u64, track: MediaStreamTrack) {
        let Some(view) = self.streams.get_mut(&sharer).filter(|view| view.serial == serial) else { return };
        match track {
            MediaStreamTrack::Video(track) => {
                let waker = Arc::clone(&self.waker);
                let wake: Waker = Arc::new(move || {
                    if let Some(waker) = waker.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref() {
                        waker();
                    }
                });
                // The screen of this app is only a preview for the person sharing it: a few frames
                // a second do, and the window does not redraw at the pace of the screen.
                let gap = if sharer == SELF_STREAM { Duration::from_millis(66) } else { Duration::ZERO };
                let store = FrameStore::screen(wake, gap);
                self.stream_videos.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(sharer.clone(), Arc::clone(&store));
                view.tasks.push(watch_track(track, store));
            }
            MediaStreamTrack::Audio(track) if sharer != SELF_STREAM => {
                let (mixer, key) = (Arc::clone(&self.mixer), stream_key(&sharer));
                view.tasks.push(tokio::spawn(async move {
                    let mut stream = NativeAudioStream::new(track, RATE as i32, 1);
                    while let Some(frame) = stream.next().await {
                        mixer.push(&key, &frame.data);
                    }
                }));
            }
            MediaStreamTrack::Audio(_) => {}
        }
    }

    fn close_stream(&mut self, sharer: &str) {
        let Some(view) = self.streams.remove(sharer) else { return };
        for task in view.tasks {
            task.abort();
        }
        view.connection.close();
        self.mixer.remove(&stream_key(sharer));
        if let Some(store) = self.stream_videos.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(sharer) {
            store.clear();
        }
    }

    /// Keeps the frames of someone's camera for the window.
    fn watch_video(&mut self, id: &str, track: RtcVideoTrack) {
        let waker = Arc::clone(&self.waker);
        let wake: Waker = Arc::new(move || {
            if let Some(waker) = waker.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref() {
                waker();
            }
        });
        let store = FrameStore::new(wake);
        self.videos.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(id.to_owned(), Arc::clone(&store));
        if let Some(old) = self.watchers.insert(id.to_owned(), watch_track(track, store)) {
            old.abort();
        }
    }

    /// Plays what arrives on a track through the mixer, and measures how loud it is.
    fn play(&self, id: &str, serial: u64, track: RtcAudioTrack) {
        let mixer = Arc::clone(&self.mixer);
        let commands = self.commands.clone();
        let id = id.to_owned();
        tokio::spawn(async move {
            let mut stream = NativeAudioStream::new(track, RATE as i32, 1);
            let mut pieces = 0u32;
            let mut loudest = 0.0f32;
            while let Some(frame) = stream.next().await {
                mixer.push(&id, &frame.data);
                loudest = loudest.max(level_of(&frame.data));
                pieces += 1;
                if pieces == 10 {
                    if commands.send(Command::Voice { id: id.clone(), level: loudest }).is_err() {
                        break;
                    }
                    pieces = 0;
                    loudest = 0.0;
                }
            }
            let _ = serial;
        });
    }

    async fn reconcile(&mut self, factory: &PeerConnectionFactory, track: &RtcAudioTrack) {
        let wanted: Vec<String> = self.participants.iter().map(|participant| participant.id.clone()).collect();
        let now = Instant::now();
        let gone: Vec<String> = self
            .peers
            .iter()
            .filter(|(id, peer)| !wanted.contains(id) && now.duration_since(peer.created) > PEER_GRACE)
            .map(|(id, _)| id.clone())
            .collect();
        for id in gone {
            self.close_peer(&id);
        }
        for id in wanted {
            if !self.peers.contains_key(&id) {
                self.create_peer(&id);
                if self.peers[&id].initiator {
                    self.start_offer(factory, track, &id).await;
                }
            }
        }
    }

    fn create_peer(&mut self, id: &str) {
        let now = Instant::now();
        self.peers.insert(
            id.to_owned(),
            Peer {
                initiator: self.self_id.as_str() < id,
                session: String::new(),
                connection: None,
                serial: 0,
                pending: Vec::new(),
                state: PeerState::Connecting,
                created: now,
                started: now,
                offered: now,
                answer_time: None,
                attempt: 0,
                connected: false,
                speaking_until: None,
                local_answer: None,
                counts: Counts::default(),
            },
        );
    }

    fn close_peer(&mut self, id: &str) {
        if let Some(peer) = self.peers.remove(id) {
            if let Some(connection) = peer.connection {
                connection.close();
            }
        }
        self.mixer.remove(id);
        if let Some(watcher) = self.watchers.remove(id) {
            watcher.abort();
        }
        self.videos.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(id);
    }

    fn set_state(&mut self, id: &str, state: PeerState) {
        if let Some(peer) = self.peers.get_mut(id) {
            if peer.state != state {
                peer.state = state;
                self.emit(CallEvent::Peer { id: id.to_owned(), state });
            }
        }
    }

    /// Throws away the connection to someone and makes a fresh one, with its callbacks.
    fn new_connection(&mut self, factory: &PeerConnectionFactory, id: &str) -> Option<PeerConnection> {
        self.serial += 1;
        let serial = self.serial;
        let mut config = RtcConfiguration::default();
        config.ice_servers = self.ice_servers.clone();
        let connection = factory.create_peer_connection(config).ok()?;
        let peer = self.peers.get_mut(id)?;
        if let Some(old) = peer.connection.take() {
            old.close();
        }
        peer.serial = serial;
        peer.attempt += 1;
        peer.started = Instant::now();
        peer.connected = false;
        peer.local_answer = None;

        let (commands, peer_id) = (self.commands.clone(), id.to_owned());
        connection.on_ice_candidate(Some(Box::new(move |candidate| {
            let _ = commands.send(Command::Candidate { id: peer_id.clone(), serial, candidate });
        })));
        let (commands, peer_id) = (self.commands.clone(), id.to_owned());
        connection.on_connection_state_change(Some(Box::new(move |state| {
            let _ = commands.send(Command::State { id: peer_id.clone(), serial, state });
        })));
        let (commands, peer_id) = (self.commands.clone(), id.to_owned());
        connection.on_track(Some(Box::new(move |event| {
            let command = match event.track {
                MediaStreamTrack::Audio(track) => Command::Track { id: peer_id.clone(), serial, track },
                MediaStreamTrack::Video(track) => Command::Video { id: peer_id.clone(), serial, track },
            };
            let _ = commands.send(command);
        })));
        peer.connection = Some(connection.clone());
        self.set_state(id, PeerState::Connecting);
        Some(connection)
    }

    async fn start_offer(&mut self, factory: &PeerConnectionFactory, track: &RtcAudioTrack, id: &str) {
        let Some(connection) = self.new_connection(factory, id) else { return };
        let session = uuid::Uuid::new_v4().to_string();
        let serial = self.serial;
        if let Some(peer) = self.peers.get_mut(id) {
            peer.session = session.clone();
            peer.pending.clear();
        }
        let both = || RtpTransceiverInit { direction: RtpTransceiverDirection::SendRecv, stream_ids: vec!["telinha".into()], send_encodings: Vec::new() };
        let video = self.video_track.clone();
        let offered = async {
            connection.add_transceiver(MediaStreamTrack::Audio(track.clone()), both()).ok()?;
            // The camera line is always offered, as the other app does, so turning a camera on
            // later needs no new negotiation: its track only starts getting frames.
            match video {
                Some(video) => connection.add_transceiver(MediaStreamTrack::Video(video), both()).ok()?,
                None => connection.add_transceiver_for_media(MediaType::Video, both()).ok()?,
            };
            // Left unset, so the lines keep the directions given above.
            let mut options = OfferOptions::default();
            options.offer_to_receive_audio = None;
            options.offer_to_receive_video = None;
            let offer = connection.create_offer(options).await.ok()?;
            let sdp = richer_opus(&offer.to_string());
            connection.set_local_description(offer).await.ok()?;
            Some(sdp)
        }
        .await;
        match offered {
            Some(sdp) => {
                if let Some(peer) = self.peers.get_mut(id).filter(|peer| peer.serial == serial) {
                    peer.offered = Instant::now();
                }
                (self.send)(id, json!({ "kind": "offer", "session": session, "description": { "type": "offer", "sdp": sdp } }));
                self.timer(id, serial, TimerKind::AwaitAnswer(0), OFFER_REPEAT_AT[0]);
            }
            None => self.schedule_retry(id, RETRY_DELAY),
        }
    }

    async fn accept_offer(&mut self, factory: &PeerConnectionFactory, track: &RtcAudioTrack, id: &str, payload: &Value) {
        let Some(sdp) = payload.pointer("/description/sdp").and_then(Value::as_str) else { return };
        let Some(session) = payload.get("session").and_then(Value::as_str) else { return };
        let Some(connection) = self.new_connection(factory, id) else { return };
        let serial = self.serial;
        if let Some(peer) = self.peers.get_mut(id) {
            peer.session = session.to_owned();
        }
        let video = self.video_track.clone();
        let answered = async {
            let offer = SessionDescription::parse(&richer_opus(sdp), SdpType::Offer).ok()?;
            connection.set_remote_description(offer).await.ok()?;
            // Takes the audio line the offer opened and sends the microphone on it.
            connection.add_track(MediaStreamTrack::Audio(track.clone()), &["telinha"]).ok()?;
            // And the camera line, both ways, as call.js turns it to "sendrecv".
            if let Some(video) = video {
                let _ = connection.add_track(MediaStreamTrack::Video(video), &["telinha"]);
            }
            Some(())
        }
        .await;
        if answered.is_none() {
            self.set_state(id, PeerState::Failed);
            return;
        }
        self.flush_candidates(id, &connection).await;
        let answer = async {
            let answer = connection.create_answer(AnswerOptions::default()).await.ok()?;
            let sdp = richer_opus(&answer.to_string());
            connection.set_local_description(answer).await.ok()?;
            Some(sdp)
        }
        .await;
        match answer {
            Some(sdp) => {
                let message = json!({ "kind": "answer", "session": session, "description": { "type": "answer", "sdp": sdp } });
                if let Some(peer) = self.peers.get_mut(id).filter(|peer| peer.serial == serial) {
                    peer.local_answer = Some(message.clone());
                }
                (self.send)(id, message);
            }
            None => self.set_state(id, PeerState::Failed),
        }
    }

    async fn flush_candidates(&mut self, id: &str, connection: &PeerConnection) {
        let Some(peer) = self.peers.get_mut(id) else { return };
        let session = peer.session.clone();
        let pending = std::mem::take(&mut peer.pending);
        for (candidate_session, candidate) in pending {
            if candidate_session == session {
                add_candidate(connection, &candidate).await;
            }
        }
    }

    async fn rtc(&mut self, factory: &PeerConnectionFactory, track: &RtcAudioTrack, from: &str, payload: Value) {
        let Some(session) = payload.get("session").and_then(Value::as_str).map(str::to_owned) else { return };
        if !self.peers.contains_key(from) {
            self.create_peer(from);
        }
        let peer = &self.peers[from];
        let same_session = peer.session == session;
        let connection = peer.connection.clone();
        match payload.get("kind").and_then(Value::as_str).unwrap_or("") {
            "offer" => {
                if peer.initiator || payload.get("description").is_none() {
                    return;
                }
                if let Some(connection) = connection.filter(|connection| same_session && connection.signaling_state() != SignalingState::Closed) {
                    // The same offer again: the answer was lost or is still on its way. It goes
                    // once more, and the connection in progress stays as it is.
                    if connection.signaling_state() == SignalingState::Stable {
                        if let Some(answer) = peer.local_answer.clone() {
                            (self.send)(from, answer);
                        }
                    }
                    return;
                }
                self.accept_offer(factory, track, from, &payload).await;
            }
            "answer" => {
                let Some(connection) = connection.filter(|connection| same_session && connection.signaling_state() == SignalingState::HaveLocalOffer) else {
                    return;
                };
                let Some(sdp) = payload.pointer("/description/sdp").and_then(Value::as_str) else { return };
                let applied = match SessionDescription::parse(&richer_opus(sdp), SdpType::Answer) {
                    Ok(answer) => connection.set_remote_description(answer).await.is_ok(),
                    Err(_) => false,
                };
                if applied {
                    if let Some(peer) = self.peers.get_mut(from) {
                        peer.answer_time = Some(peer.offered.elapsed());
                    }
                    self.flush_candidates(from, &connection).await;
                } else {
                    self.schedule_retry(from, RETRY_DELAY);
                }
            }
            "candidate" => {
                let Some(candidate) = payload.get("candidate").filter(|candidate| candidate.is_object()).cloned() else { return };
                match connection.filter(|connection| same_session && connection.current_remote_description().is_some()) {
                    Some(connection) => add_candidate(&connection, &candidate).await,
                    None => {
                        if let Some(peer) = self.peers.get_mut(from) {
                            peer.pending.push((session, candidate));
                            if peer.pending.len() > MAX_PENDING_CANDIDATES {
                                peer.pending.remove(0);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn state_changed(&mut self, id: &str, serial: u64, state: PeerConnectionState) {
        let Some(peer) = self.peers.get_mut(id).filter(|peer| peer.serial == serial) else { return };
        let took = format!("{:.1} s", peer.started.elapsed().as_secs_f32());
        let attempt = peer.attempt;
        match state {
            PeerConnectionState::Connected => {
                let first = !peer.connected;
                peer.connected = true;
                self.set_state(id, PeerState::Connected);
                if first {
                    self.emit(CallEvent::Report(format!("conectou com {} em {took}, tentativa {attempt}", self.name_of(id))));
                }
            }
            PeerConnectionState::Failed => {
                let what = if peer.connected { "caiu" } else { "nao conectou" };
                self.emit(CallEvent::Report(format!("{what} com {} em {took}, tentativa {attempt}", self.name_of(id))));
                self.schedule_retry(id, DROP_RETRY_DELAY);
            }
            PeerConnectionState::Disconnected => {
                // A connection is only given up as lost some 10 s after going silent. When the
                // negotiation with the person is quick, redoing it costs less than waiting.
                let quick = peer.answer_time.is_some_and(|time| time < QUICK_ANSWER);
                if peer.initiator && quick {
                    self.timer(id, serial, TimerKind::Drop, DROP_GRACE);
                }
            }
            _ => {}
        }
    }

    fn schedule_retry(&mut self, id: &str, delay: Duration) {
        self.set_state(id, PeerState::Failed);
        let Some(peer) = self.peers.get(id) else { return };
        if peer.initiator {
            self.timer(id, peer.serial, TimerKind::Retry, delay);
        }
    }

    async fn timer_fired(&mut self, factory: &PeerConnectionFactory, track: &RtcAudioTrack, id: &str, serial: u64, kind: TimerKind) {
        let Some(peer) = self.peers.get(id).filter(|peer| peer.serial == serial) else { return };
        let Some(connection) = peer.connection.clone() else { return };
        let in_room = self.participants.iter().any(|participant| participant.id == id);
        match kind {
            // Without an answer the connection never gets to fail, and an offer or an answer
            // lost on the way would leave the person in "connecting" forever. While the answer
            // does not come the same offer goes again, further and further apart, to whoever
            // knows how to answer again without starting over. At the end the connection starts
            // from scratch.
            TimerKind::AwaitAnswer(step) => {
                if connection.signaling_state() != SignalingState::HaveLocalOffer {
                    return;
                }
                let repeats = self.participants.iter().any(|participant| participant.id == id && participant.answers_repeats);
                if repeats {
                    if let Some(offer) = connection.current_local_description().map(|description| description.to_string()).or_else(|| None) {
                        (self.send)(id, json!({ "kind": "offer", "session": peer.session, "description": { "type": "offer", "sdp": offer } }));
                    }
                }
                let since = peer.offered.elapsed();
                let (next, at) = match OFFER_REPEAT_AT.get(step + 1) {
                    Some(at) => (TimerKind::AwaitAnswer(step + 1), *at),
                    None if step + 1 == OFFER_REPEAT_AT.len() => (TimerKind::AwaitAnswer(step + 1), ANSWER_TIMEOUT),
                    None => {
                        self.schedule_retry(id, RETRY_DELAY);
                        return;
                    }
                };
                self.timer(id, serial, next, at.saturating_sub(since));
            }
            TimerKind::Drop => {
                if connection.connection_state() == PeerConnectionState::Disconnected {
                    self.schedule_retry(id, DROP_RETRY_DELAY);
                }
            }
            TimerKind::Retry => {
                // The connection may have come back by itself while the wait ran.
                if connection.connection_state() != PeerConnectionState::Connected && in_room {
                    self.start_offer(factory, track, id).await;
                }
            }
        }
    }
}

async fn add_candidate(connection: &PeerConnection, candidate: &Value) {
    let Some(sdp) = candidate.get("candidate").and_then(Value::as_str).filter(|sdp| !sdp.is_empty()) else { return };
    let mid = candidate.get("sdpMid").and_then(Value::as_str).unwrap_or("0");
    let index = candidate.get("sdpMLineIndex").and_then(Value::as_i64).unwrap_or(0) as i32;
    // A candidate that is old or malformed is skipped; the connection goes on with the others.
    if let Ok(candidate) = IceCandidate::parse(mid, index, sdp) {
        let _ = connection.add_ice_candidate(candidate).await;
    }
}

/// The voice at 96 kbit/s instead of the 32 WebRTC picks for one channel. At 32 the encoder
/// treats everything as speech, and the sounds of the soundboard come out watery, as if noise
/// suppression had gone over them. Written in every description sent and read, so each side
/// sends the richer voice even to an app that does not ask for it.
pub(crate) fn richer_opus(sdp: &str) -> String {
    const RATE: &str = "maxaveragebitrate=96000";
    let opus: Vec<String> = sdp
        .lines()
        .filter_map(|line| line.strip_prefix("a=rtpmap:"))
        .filter_map(|rest| rest.split_once(' '))
        .filter(|(_, codec)| codec.to_ascii_lowercase().starts_with("opus/"))
        .map(|(kind, _)| kind.to_owned())
        .collect();
    let ending = if sdp.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out: Vec<String> = Vec::new();
    for line in sdp.lines() {
        let fmtp = opus.iter().find_map(|kind| line.strip_prefix(&format!("a=fmtp:{kind} ")).map(|params| (kind, params)));
        match fmtp {
            Some((kind, params)) => {
                let kept: Vec<&str> = params.split(';').map(str::trim).filter(|param| !param.is_empty() && !param.starts_with("maxaveragebitrate=")).collect();
                out.push(format!("a=fmtp:{kind} {};{RATE}", kept.join(";")));
            }
            None => out.push(line.to_owned()),
        }
    }
    let mut text = out.join(ending);
    if sdp.ends_with('\n') {
        text.push_str(ending);
    }
    text
}

#[cfg(test)]
mod opus_tests {
    use super::richer_opus;

    #[test]
    fn the_opus_line_asks_for_the_richer_voice_and_nothing_else_changes() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111 63\r\na=rtpmap:111 opus/48000/2\r\na=fmtp:111 minptime=10;useinbandfec=1\r\na=rtpmap:63 red/48000/2\r\na=fmtp:63 111/111\r\n";
        let rich = richer_opus(sdp);
        assert!(rich.contains("a=fmtp:111 minptime=10;useinbandfec=1;maxaveragebitrate=96000\r\n"), "{rich}");
        assert!(rich.contains("a=fmtp:63 111/111\r\n"));
        assert_eq!(rich.matches("\r\n").count(), sdp.matches("\r\n").count());
        // Twice is the same as once, and a rate already there is replaced.
        assert_eq!(richer_opus(&rich), rich);
        assert!(richer_opus(&sdp.replace("useinbandfec=1", "maxaveragebitrate=20000")).contains("a=fmtp:111 minptime=10;maxaveragebitrate=96000"));
    }
}
