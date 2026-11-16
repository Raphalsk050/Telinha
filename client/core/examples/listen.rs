//! Listens to the servers of a real profile without publishing anything, and prints what goes
//! by: one line per message, with the broker it came through.
//!
//!   cargo run -p telinha-core --example listen -- <data dir> <seconds>

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, Transport};
use serde_json::Value;
use telinha_core::envelope::open_message;
use telinha_core::servers::ServerStore;
use telinha_core::signaling::{TOPIC_PREFIX, default_brokers};
use telinha_core::vault::{OsVault, Vault};

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

fn clock() -> String {
    let ms = now_ms();
    format!("{:02}:{:02}:{:02}.{:03}", (ms / 3_600_000) % 24, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000)
}

fn short(text: &str) -> &str {
    &text[..4.min(text.len())]
}

#[tokio::main]
async fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let data_dir = std::path::PathBuf::from(&arguments[1]);
    let seconds: u64 = arguments.get(2).and_then(|text| text.parse().ok()).unwrap_or(60);
    let vault: Arc<dyn Vault> = Arc::new(OsVault::open(&data_dir).expect("vault"));
    let mut store = ServerStore::load(data_dir.join("servers.json"), vault);
    let mut spaces = Vec::new();
    for server in store.list() {
        if let Some(keys) = store.keys_for(&server.id) {
            println!("server {} = {}", server.name, keys.id);
            spaces.push((server.name.clone(), keys));
        }
    }
    let spaces = Arc::new(spaces);
    for broker in default_brokers() {
        let spaces = Arc::clone(&spaces);
        tokio::spawn(async move {
            let url = url::Url::parse(&broker.url).unwrap();
            let port = url.port().unwrap_or(443);
            let bare = format!("wss://{}:{port}{}", url.host_str().unwrap(), url.path());
            let name = url.host_str().unwrap().to_owned();
            let mut options = MqttOptions::new(format!("ouvinte-{}", now_ms() % 1_000_000_007), bare, port);
            options.set_transport(Transport::wss_with_default_config());
            options.set_keep_alive(Duration::from_secs(30));
            options.set_max_packet_size(2 * 1024 * 1024, 2 * 1024 * 1024);
            if !url.username().is_empty() {
                options.set_credentials(url.username(), url.password().unwrap_or(""));
            }
            let (client, mut eventloop) = AsyncClient::new(options, 64);
            eventloop.network_options.set_connection_timeout(30);
            let started = std::time::Instant::now();
            loop {
                match eventloop.poll().await {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        println!("{} {name} conectado em {:.1} s", clock(), started.elapsed().as_secs_f32());
                        for (_, keys) in spaces.iter() {
                            let _ = client.subscribe(format!("{TOPIC_PREFIX}{}", keys.id), QoS::AtMostOnce).await;
                        }
                    }
                    Ok(Event::Incoming(Packet::Publish(publish))) => {
                        let Some(space) = publish.topic.strip_prefix(TOPIC_PREFIX) else { continue };
                        let Some((server, keys)) = spaces.iter().find(|(_, keys)| keys.id == space) else { continue };
                        let Ok(text) = std::str::from_utf8(&publish.payload) else { continue };
                        let Some(message) = open_message(&keys.key, text, now_ms()) else {
                            println!("{} {name} [{server}] nao abriu", clock());
                            continue;
                        };
                        let field = |key: &str| message.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
                        let kind = field("type");
                        let from = field("from");
                        let at = message.get("at").and_then(Value::as_i64).unwrap_or(0);
                        let lag = now_ms() - at;
                        let detail = match kind.as_str() {
                            "presence" => format!(
                                "{} online={} voice={}",
                                field("name"),
                                message.get("online").map(Value::to_string).unwrap_or_default(),
                                message.get("voice").map(Value::to_string).unwrap_or_default()
                            ),
                            "rtc" => {
                                let payload = message.get("payload").cloned().unwrap_or(Value::Null);
                                let what = payload.get("kind").and_then(Value::as_str).unwrap_or("?").to_owned();
                                let session = payload.get("session").and_then(Value::as_str).unwrap_or("").to_owned();
                                format!("to={} room={} {what} session={} bytes={}", short(&field("to")), field("roomId"), short(&session), payload.to_string().len())
                            }
                            _ => String::new(),
                        };
                        println!("{} {name} [{server}] {kind} from={} lag={lag}ms {detail}", clock(), short(&from));
                    }
                    Ok(_) => {}
                    Err(error) => {
                        println!("{} {name} erro: {error}", clock());
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        });
    }
    tokio::time::sleep(Duration::from_secs(seconds)).await;
}
