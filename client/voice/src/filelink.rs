//! The direct connection a large file travels on: a WebRTC data channel between the two
//! computers. Who receives opens the channel and makes the offer; who has the file answers and
//! pours the pieces in. Each side sends its whole description at once, candidates included, so
//! one message each way through the message servers is enough. Same as
//! `desktop/renderer/files.js`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use libwebrtc::data_channel::{DataChannel, DataChannelInit, DataChannelState};
use libwebrtc::ice_candidate::IceCandidate;
use libwebrtc::peer_connection::{AnswerOptions, IceGatheringState, OfferOptions, PeerConnection, PeerConnectionState};
use libwebrtc::peer_connection_factory::{IceServer, PeerConnectionFactory, RtcConfiguration};
use libwebrtc::session_description::{SdpType, SessionDescription};
use telinha_core::hub::{FileLink, LinkFuture, Receiving, Serving};
use tokio::sync::{Notify, mpsc, oneshot};

const GATHER_TIMEOUT: Duration = Duration::from_secs(3);
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
/// Above this much waiting in the channel, the sender stops and lets it drain.
const HIGH_WATER_BYTES: u64 = 4 * 1024 * 1024;
/// Who receives closes the connection when the last byte is in. If that never comes, the
/// sending side goes away by itself after this long.
const LINGER: Duration = Duration::from_secs(30);

pub struct WebRtcLink {
    factory: PeerConnectionFactory,
    ice_servers: Vec<IceServer>,
}

impl WebRtcLink {
    pub fn new(ice_servers: Vec<IceServer>) -> Self {
        Self { factory: PeerConnectionFactory::default(), ice_servers }
    }

    fn connection(&self) -> Result<PeerConnection, String> {
        let mut config = RtcConfiguration::default();
        config.ice_servers = self.ice_servers.clone();
        self.factory.create_peer_connection(config).map_err(|error| error.to_string())
    }
}

/// Collects the candidates of a connection as they are found, and tells when there are no more.
pub(crate) struct Gathering {
    found: Arc<Mutex<Vec<IceCandidate>>>,
    complete: Arc<Notify>,
}

impl Gathering {
    pub(crate) fn watch(connection: &PeerConnection) -> Self {
        let found = Arc::new(Mutex::new(Vec::new()));
        let complete = Arc::new(Notify::new());
        let list = Arc::clone(&found);
        connection.on_ice_candidate(Some(Box::new(move |candidate| {
            list.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(candidate);
        })));
        let done = Arc::clone(&complete);
        connection.on_ice_gathering_state_change(Some(Box::new(move |state| {
            if state == IceGatheringState::Complete {
                done.notify_one();
            }
        })));
        Self { found, complete }
    }

    /// Waits for the candidates and writes them into the description, which is how the other
    /// side gets them without a message for each.
    pub(crate) async fn finish(self, mut sdp: String) -> String {
        // The end of the search is not always announced, so a search that found something and
        // then went quiet for a moment also counts as over.
        let count = || self.found.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).len();
        let quiet = async {
            let mut seen = 0;
            let mut still = 0;
            while still < 4 || seen == 0 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let now = count();
                still = if now == seen { still + 1 } else { 0 };
                seen = now;
            }
        };
        tokio::select! {
            () = self.complete.notified() => {}
            () = quiet => {}
            () = tokio::time::sleep(GATHER_TIMEOUT) => {}
        }
        if !sdp.ends_with('\n') {
            sdp.push_str("\r\n");
        }
        for candidate in self.found.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter() {
            let line = candidate.candidate();
            let line = line.trim();
            if !line.is_empty() {
                sdp.push_str(if line.starts_with("a=") { "" } else { "a=" });
                sdp.push_str(line);
                sdp.push_str("\r\n");
            }
        }
        sdp.push_str("a=end-of-candidates\r\n");
        sdp
    }
}

/// Tells when a channel changes state, and lets a task wait for it.
fn watch_state(channel: &DataChannel) -> Arc<Notify> {
    let changed = Arc::new(Notify::new());
    let tell = Arc::clone(&changed);
    channel.on_state_change(Some(Box::new(move |_| tell.notify_waiters())));
    changed
}

async fn wait_open(channel: &DataChannel, changed: &Notify) -> bool {
    let wait = async {
        loop {
            match channel.state() {
                DataChannelState::Open => return true,
                DataChannelState::Closing | DataChannelState::Closed => return false,
                DataChannelState::Connecting => {
                    let _ = tokio::time::timeout(Duration::from_millis(100), changed.notified()).await;
                }
            }
        }
    };
    tokio::time::timeout(OPEN_TIMEOUT, wait).await.unwrap_or(false)
}

impl FileLink for WebRtcLink {
    fn receive(&self) -> LinkFuture<Receiving> {
        let connection = self.connection();
        Box::pin(async move {
            let connection = connection?;
            let channel = connection.create_data_channel("file", DataChannelInit::default()).map_err(|error| error.to_string())?;
            // The pieces leave the thread of the connection at once and wait here for the disk.
            let (arrived, mut queue) = mpsc::unbounded_channel::<Vec<u8>>();
            let arrived = Arc::new(Mutex::new(Some(arrived)));
            let into = Arc::clone(&arrived);
            channel.on_message(Some(Box::new(move |buffer| {
                if buffer.binary {
                    if let Some(queue) = into.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref() {
                        let _ = queue.send(buffer.data.to_vec());
                    }
                }
            })));
            let changed = watch_state(&channel);
            let failed = Arc::clone(&changed);
            connection.on_connection_state_change(Some(Box::new(move |state| {
                if matches!(state, PeerConnectionState::Failed | PeerConnectionState::Closed) {
                    failed.notify_waiters();
                }
            })));

            let gathering = Gathering::watch(&connection);
            let offer = connection.create_offer(OfferOptions::default()).await.map_err(|error| error.to_string())?;
            let sdp = offer.to_string();
            connection.set_local_description(offer).await.map_err(|error| error.to_string())?;
            let offer = gathering.finish(sdp).await;

            let (answer, answered) = oneshot::channel::<String>();
            let (pieces, taken) = mpsc::channel::<Vec<u8>>(64);
            tokio::spawn(async move {
                let connected = async {
                    let sdp = answered.await.ok()?;
                    let answer = SessionDescription::parse(&sdp, SdpType::Answer).ok()?;
                    connection.set_remote_description(answer).await.ok()
                };
                if connected.await.is_some() {
                    // Hands the pieces over until the file is whole on the other end of this
                    // function, the channel closes or the connection fails.
                    loop {
                        tokio::select! {
                            piece = queue.recv() => match piece {
                                Some(piece) => {
                                    if pieces.send(piece).await.is_err() {
                                        break;
                                    }
                                }
                                None => break,
                            },
                            () = pieces.closed() => break,
                            () = changed.notified() => {
                                let gone = matches!(channel.state(), DataChannelState::Closing | DataChannelState::Closed)
                                    || matches!(connection.connection_state(), PeerConnectionState::Failed | PeerConnectionState::Closed);
                                if gone {
                                    // What already arrived is still handed over before the end.
                                    arrived.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
                                }
                            }
                        }
                    }
                }
                channel.on_message(None);
                channel.close();
                connection.close();
            });
            Ok(Receiving { offer, answer, pieces: taken })
        })
    }

    fn serve(&self, offer: String) -> LinkFuture<Serving> {
        let connection = self.connection();
        Box::pin(async move {
            let connection = connection?;
            let (opened, mut channels) = mpsc::unbounded_channel::<DataChannel>();
            connection.on_data_channel(Some(Box::new(move |channel| {
                let _ = opened.send(channel);
            })));
            let gathering = Gathering::watch(&connection);
            let offer = SessionDescription::parse(&offer, SdpType::Offer).map_err(|error| error.to_string())?;
            connection.set_remote_description(offer).await.map_err(|error| error.to_string())?;
            let answer = connection.create_answer(AnswerOptions::default()).await.map_err(|error| error.to_string())?;
            let sdp = answer.to_string();
            connection.set_local_description(answer).await.map_err(|error| error.to_string())?;
            let answer = gathering.finish(sdp).await;

            let (pieces, mut waiting) = mpsc::channel::<Vec<u8>>(16);
            tokio::spawn(async move {
                let pour = async {
                    let channel = tokio::time::timeout(OPEN_TIMEOUT, channels.recv()).await.ok()??;
                    let changed = watch_state(&channel);
                    if !wait_open(&channel, &changed).await {
                        return None;
                    }
                    let drained = Arc::new(Notify::new());
                    let tell = Arc::clone(&drained);
                    channel.on_buffered_amount_change(Some(Box::new(move |_| tell.notify_one())));
                    while let Some(piece) = waiting.recv().await {
                        while channel.buffered_amount() > HIGH_WATER_BYTES && channel.state() == DataChannelState::Open {
                            let _ = tokio::time::timeout(Duration::from_millis(50), drained.notified()).await;
                        }
                        if channel.state() != DataChannelState::Open || channel.send(&piece, true).is_err() {
                            return Some(channel);
                        }
                    }
                    // Everything was handed to the channel. The other side closes it when the
                    // last byte arrives there.
                    let _ = tokio::time::timeout(LINGER, async {
                        while channel.state() == DataChannelState::Open {
                            let _ = tokio::time::timeout(Duration::from_millis(200), changed.notified()).await;
                        }
                    })
                    .await;
                    Some(channel)
                };
                if let Some(channel) = pour.await {
                    channel.on_buffered_amount_change(None);
                    channel.close();
                }
                // Stops whoever is still reading the file from the disk.
                waiting.close();
                connection.close();
            });
            Ok(Serving { answer, pieces })
        })
    }
}
