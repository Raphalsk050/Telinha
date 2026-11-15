//! A hub and a call with a tone for a microphone, for `voice_interop.js` to run two of and check
//! that they connect and hear each other through the real message servers.
//!
//!   cargo run -p telinha-voice --example voice_probe -- <data folder> <name> <seconds> [invite]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use telinha_core::hub::{Hub, HubEvent};
use telinha_core::servers::ChannelKind;
use telinha_core::signaling::default_brokers;
use telinha_voice::{Call, CallEvent, Participant, PeerState, Sound, default_ice_servers};

fn say(value: Value) {
    println!("{value}");
}

fn participants(hub: &Hub, space_id: &str, room_id: &str) -> Vec<Participant> {
    hub.peers(space_id)
        .into_iter()
        .filter_map(|peer| {
            let voice = peer.voice.as_ref().filter(|voice| voice.room_id == room_id)?;
            Some(Participant { answers_repeats: voice.version >= 2, name: peer.name.clone(), id: peer.id })
        })
        .collect()
}

#[tokio::main]
async fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let folder = PathBuf::from(&arguments[1]);
    let name = arguments[2].clone();
    let seconds: u64 = arguments[3].parse().unwrap_or(40);
    let invite = arguments.get(4).cloned();

    let (hub, mut events) = Hub::open(&folder, default_brokers());
    hub.set_name(&name);
    let server_id = match &invite {
        Some(invite) => hub.join_server(invite).expect("a valid invite").0.id,
        None => {
            let server = hub.create_server("Chamada de teste");
            say(json!({ "event": "ready", "invite": hub.invite_code(&server.id) }));
            server.id
        }
    };
    hub.start();

    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    let mut call: Option<(Call, String)> = None;
    let (forward, mut call_events) = tokio::sync::mpsc::unbounded_channel::<CallEvent>();
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            Some(event) = call_events.recv() => match event {
                CallEvent::Peer { state, .. } => say(json!({ "event": "peer", "connected": state == PeerState::Connected })),
                CallEvent::Speaking { id, speaking } if id != hub.instance_id() => say(json!({ "event": "heard", "speaking": speaking })),
                CallEvent::Report(line) => say(json!({ "event": "report", "line": line })),
                _ => {}
            },
            Some(event) = events.recv() => match event {
                // The one who joined by invite learns the rooms from the other's presence.
                HubEvent::Servers | HubEvent::Peers { .. } if call.is_none() => {
                    let room = hub.server(&server_id).and_then(|server| {
                        server.channels.iter().find(|channel| channel.kind == ChannelKind::Voice).map(|channel| channel.id.clone())
                    });
                    let Some(room_id) = room else { continue };
                    if hub.join_voice(&server_id, &room_id, true, false, false).is_none() {
                        continue;
                    }
                    let (sender, space) = (Arc::clone(&hub), server_id.clone());
                    let (started, mut from_call) = Call::start(hub.instance_id(), default_ice_servers(), Sound::Tone, true, false, move |to, payload| {
                        sender.send_rtc(&space, to, &payload);
                    });
                    let forward = forward.clone();
                    tokio::spawn(async move {
                        while let Some(event) = from_call.recv().await {
                            let _ = forward.send(event);
                        }
                    });
                    started.set_participants(participants(&hub, &server_id, &room_id));
                    say(json!({ "event": "joined" }));
                    call = Some((started, room_id));
                }
                HubEvent::Peers { .. } => {
                    if let Some((call, room_id)) = &call {
                        call.set_participants(participants(&hub, &server_id, room_id));
                    }
                }
                HubEvent::Rtc { from, payload, .. } => {
                    if let Some((call, _)) = &call {
                        call.handle_rtc(&from, payload);
                    }
                }
                _ => {}
            },
        }
    }
    if let Some((call, _)) = &call {
        call.stop();
    }
    hub.leave_voice();
    hub.stop().await;
    say(json!({ "event": "stopped" }));
}
