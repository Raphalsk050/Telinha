//! A whole hub in a scratch folder, for `hub_interop.js` to play the Electron app against.
//!
//!   cargo run -p telinha-core --example hub_probe -- <data folder> <seconds>
//!
//! Creates a server, prints its invite, and then prints one JSON line per thing that happens.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use telinha_core::hub::{Conversation, Hub, HubEvent, Outgoing, OutgoingFile};
use telinha_core::servers::ChannelKind;
use telinha_core::signaling::default_brokers;
use telinha_core::transfers::sha256_hex;

fn say(value: Value) {
    println!("{value}");
}

#[tokio::main]
async fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let folder = PathBuf::from(&arguments[1]);
    let seconds: u64 = arguments.get(2).and_then(|text| text.parse().ok()).unwrap_or(40);

    let (hub, mut events) = Hub::open(&folder, default_brokers());
    hub.set_name("Rust");
    let server = hub.create_server("Servidor de teste");
    let text_channel = server.channels.iter().find(|channel| channel.kind == ChannelKind::Text).unwrap().id.clone();
    let conversation = Conversation::channel(&server.id, &text_channel);
    hub.set_viewing(Some(&conversation));
    say(json!({
        "event": "ready", "invite": hub.invite_code(&server.id), "server": server.id, "channel": text_channel,
        "member": hub.member_id(),
    }));
    hub.start();

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
            HubEvent::Peers { space_id } => {
                let peers = hub.peers(&space_id);
                say(json!({ "event": "peers", "names": peers.iter().map(|peer| peer.name.clone()).collect::<Vec<_>>() }));
                if !peers.is_empty() && !greeted {
                    greeted = true;
                    let sent = hub.send_chat(&conversation, Outgoing { text: "oi do hub em Rust".into(), ..Outgoing::default() });
                    say(json!({ "event": "sent-text", "ok": sent.is_ok_and(|message| message.is_some()) }));
                    let data: Vec<u8> = (0..600_000u32).map(|value| (value % 253) as u8).collect();
                    let file = OutgoingFile { name: "dados.bin".into(), mime: "application/octet-stream".into(), data, path: None };
                    let sent = hub.send_chat(&conversation, Outgoing { file: Some(file), ..Outgoing::default() });
                    say(json!({ "event": "sent-file", "ok": sent.is_ok_and(|message| message.is_some()) }));
                }
            }
            HubEvent::ChatMessage { conversation, message } => {
                let image = message.image.as_ref().map(|image| {
                    let stored = hub.chat_image(&conversation, &image.id);
                    json!({ "width": image.width, "height": image.height, "bytes": stored.map(|(_, data)| data.len()) })
                });
                let file = message.file.as_ref().map(|file| {
                    let stored = hub.chat_file(&conversation, &file.id);
                    json!({ "name": file.name, "size": file.size, "sha256": stored.map(|data| sha256_hex(&data)) })
                });
                say(json!({
                    "event": "chat", "id": message.id, "author": message.author, "text": message.text,
                    "image": image, "file": file, "embed": message.embed.as_ref().map(|embed| embed.title.clone()),
                    "stored": hub.history(&conversation).len(),
                }));
            }
            HubEvent::ChatUpdate { message, .. } => {
                say(json!({ "event": "update", "id": message.id, "delivered": message.delivered }));
            }
            HubEvent::ChatRemove { conversation, message_id } => {
                say(json!({ "event": "removed", "id": message_id, "stored": hub.history(&conversation).len() }));
            }
            HubEvent::Servers => {
                let server = hub.server(&server.id).unwrap();
                say(json!({
                    "event": "servers",
                    "members": server.members.iter().map(|member| member.name.clone()).collect::<Vec<_>>(),
                    "channels": server.channels.iter().map(|channel| channel.name.clone()).collect::<Vec<_>>(),
                }));
            }
            HubEvent::Picture { member_id, banner } => {
                let picture = hub.picture(&member_id, banner);
                say(json!({ "event": "picture", "banner": banner, "bytes": picture.map(|picture| picture.data.len()) }));
            }
            HubEvent::Notify { title, body, .. } => say(json!({ "event": "notify", "title": title, "body": body })),
            HubEvent::UploadProgress { done, total, .. } => say(json!({ "event": "progress", "done": done, "total": total })),
            _ => {}
        }
    }
    hub.stop().await;
    say(json!({ "event": "stopped" }));
}
