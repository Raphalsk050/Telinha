//! A hub that joins a voice room, shares a monitor and watches whoever else shares, for
//! `stream_interop.js` to play the Electron app against.
//!
//!   cargo run -p telinha-core --example stream_probe -- <data folder> <monitor handle> <seconds>

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use telinha_core::engine::{ShareAudio, TargetKind};
use telinha_core::hub::{Hub, HubEvent, ShareChoice};
use telinha_core::servers::ChannelKind;
use telinha_core::signaling::default_brokers;

fn say(value: Value) {
    println!("{value}");
}

#[tokio::main]
async fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let folder = PathBuf::from(&arguments[1]);
    let monitor: u64 = arguments[2].parse().expect("monitor handle");
    let seconds: u64 = arguments.get(3).and_then(|text| text.parse().ok()).unwrap_or(60);

    let (hub, mut events) = Hub::open(&folder, default_brokers());
    hub.set_name("Rust");
    let server = hub.create_server("Transmissoes de teste");
    let room = server.channels.iter().find(|channel| channel.kind == ChannelKind::Voice).unwrap().id.clone();
    hub.join_voice(&server.id, &room, true, false, false).expect("the room exists");
    say(json!({ "event": "ready", "invite": hub.invite_code(&server.id), "server": server.id, "room": room, "instance": hub.instance_id() }));
    hub.start();

    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    let mut shared = false;
    loop {
        let event = tokio::select! {
            event = events.recv() => event,
            _ = &mut deadline => break,
        };
        let Some(event) = event else { break };
        match event {
            HubEvent::Peers { space_id } => {
                let peers = hub.peers(&space_id);
                if !peers.is_empty() && !shared {
                    shared = true;
                    let choice = ShareChoice { kind: TargetKind::Monitor, handle: monitor, name: "Monitor".into(), audio: ShareAudio::None, fps: None };
                    say(json!({ "event": "go-live", "result": hub.go_live(choice).err() }));
                }
                for peer in peers.iter().filter(|peer| peer.voice.as_ref().is_some_and(|voice| voice.room_id == room && voice.live)) {
                    if !hub.streams().incoming.iter().any(|entry| entry.sharer_id == peer.id) {
                        say(json!({ "event": "watch", "peer": peer.name, "result": hub.watch(&peer.id).err() }));
                    }
                }
            }
            HubEvent::Streams => {
                let view = hub.streams();
                say(json!({
                    "event": "streams",
                    "outgoing": view.outgoing.as_ref().map(|outgoing| json!({
                        "phase": format!("{:?}", outgoing.phase), "width": outgoing.width,
                        "viewers": outgoing.viewers.iter().map(|(_, state)| state.clone()).collect::<Vec<_>>(),
                    })),
                    "incoming": view.incoming.iter().map(|entry| json!({ "name": entry.name, "phase": format!("{:?}", entry.phase) })).collect::<Vec<_>>(),
                }));
            }
            HubEvent::EngineEvent { source, event } if event["event"] == "stats" => {
                say(json!({ "event": "stats", "source": if source == "outgoing" { "outgoing" } else { "incoming" }, "frames": event.get("frames"), "presented": event.get("presented") }));
            }
            HubEvent::OutgoingEnded { code, stopped, error } => say(json!({ "event": "outgoing-ended", "code": code, "stopped": stopped, "error": error })),
            HubEvent::IncomingEnded { name, reason, .. } => say(json!({ "event": "incoming-ended", "name": name, "reason": format!("{reason:?}") })),
            _ => {}
        }
    }
    hub.leave_voice();
    tokio::time::sleep(Duration::from_secs(5)).await;
    while let Ok(event) = events.try_recv() {
        if let HubEvent::OutgoingEnded { code, stopped, .. } = event {
            say(json!({ "event": "outgoing-ended", "code": code, "stopped": stopped }));
        }
    }
    hub.stop().await;
    say(json!({ "event": "stopped" }));
}
