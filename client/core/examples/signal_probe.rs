//! Talks to the JavaScript signaling through the real message servers, with a throwaway key.
//!
//!   cargo run -p telinha-core --example signal_probe -- <key in hex> <space id> <seconds>
//!
//! Prints one JSON line per thing that happens, for `interop.js` to read.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use telinha_core::signaling::{Broker, Identity, SignalEvent, Signaling, default_brokers};
use tokio::sync::mpsc;

struct Probe {
    space_id: String,
    key: [u8; 32],
}

impl Identity for Probe {
    fn space_ids(&self) -> Vec<String> {
        vec![self.space_id.clone()]
    }

    fn key_for(&self, space_id: &str) -> Option<[u8; 32]> {
        (space_id == self.space_id).then_some(self.key)
    }

    fn profile_name(&self) -> String {
        "Rust".into()
    }

    fn presence_for(&self, _space_id: &str) -> Map<String, Value> {
        let mut extra = Map::new();
        extra.insert("memberId".into(), "f".repeat(32).into());
        extra.insert("style".into(), json!({ "name": null, "theme": null, "bio": "feito em Rust" }));
        extra
    }
}

fn say(value: Value) {
    println!("{value}");
}

#[tokio::main]
async fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let key: [u8; 32] = hex::decode(&arguments[1]).expect("key in hex").try_into().expect("32 bytes");
    let space_id = arguments[2].clone();
    let seconds: u64 = arguments.get(3).and_then(|text| text.parse().ok()).unwrap_or(25);
    let brokers: Vec<Broker> = match std::env::var("PROBE_BROKERS") {
        Ok(list) => list.split(',').map(Broker::new).collect(),
        Err(_) => default_brokers(),
    };

    let (sender, mut events) = mpsc::unbounded_channel();
    let signaling = Signaling::new(Arc::new(Probe { space_id: space_id.clone(), key }), brokers, sender);
    signaling.start();
    say(json!({ "event": "started", "instance": signaling.instance_id() }));

    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    let mut greeted = false;
    loop {
        let event = tokio::select! {
            event = events.recv() => event,
            _ = &mut deadline => break,
        };
        let Some(event) = event else { break };
        match event {
            SignalEvent::Status(status) => say(json!({ "event": "status", "brokers": status.brokers })),
            SignalEvent::Peers { peers, .. } => {
                let list: Vec<Value> = peers
                    .iter()
                    .map(|peer| json!({ "name": peer.name, "memberId": peer.member_id, "voice": peer.voice.as_ref().map(|voice| voice.room_id.clone()) }))
                    .collect();
                say(json!({ "event": "peers", "peers": list }));
                if !peers.is_empty() && !greeted {
                    greeted = true;
                    let small = signaling.publish(&space_id, "chat", json!({ "messageId": "from-rust", "text": "olá do Rust, ação 😀" }));
                    let acked_space = space_id.clone();
                    let large = signaling.publish_acked(
                        &acked_space,
                        "file-chunk",
                        json!({ "transferId": "probe-transfer", "index": 0, "total": 1, "data": "A".repeat(200 * 1024) }),
                        |delivered| say(json!({ "event": "acked", "delivered": delivered })),
                    );
                    say(json!({ "event": "sent", "small": small, "large": large }));
                }
            }
            SignalEvent::Message { message, .. } => {
                let size = message.to_string().len();
                say(json!({ "event": "message", "type": message["type"], "text": message.get("text"), "size": size }));
            }
            SignalEvent::Online { online, .. } => say(json!({ "event": "online", "online": online })),
            SignalEvent::PeerName { .. } | SignalEvent::PeerPresence { .. } => {}
        }
    }
    signaling.stop().await;
    say(json!({ "event": "stopped" }));
}
