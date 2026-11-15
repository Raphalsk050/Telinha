//! Puts a call of this app face to face with a browser, which is what the Electron app talks
//! with, to check that the two negotiate and hear each other.
//!
//!   cargo run -p telinha-voice --example browser_probe -- <port> <id of this side> <seconds>
//!
//! Serves `browser_probe.html` on the port and carries the negotiation messages between the page
//! and the call over plain HTTP. The page runs the same steps as `desktop/renderer/call.js`.
//! Prints one JSON line per thing that happens, and what the page reports about itself.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use telinha_voice::{Call, CallEvent, Participant, PeerState, Sound, default_ice_servers};

const PAGE: &str = include_str!("browser_probe.html");

fn say(value: Value) {
    println!("{value}");
}

#[tokio::main]
async fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let port: u16 = arguments[1].parse().expect("port");
    let self_id = arguments[2].clone();
    let seconds: u64 = arguments[3].parse().unwrap_or(30);

    // What the call wants to tell the page waits here until the page asks for it.
    let outbox: Arc<Mutex<Vec<Value>>> = Arc::default();
    let to_page = Arc::clone(&outbox);
    let (call, mut events) = Call::start(&self_id, default_ice_servers(), Sound::Tone, true, false, move |_to, payload| {
        to_page.lock().unwrap().push(payload);
    });
    call.set_participants(vec![Participant { id: "page".into(), answers_repeats: true, name: "navegador".into() }]);

    let listener = TcpListener::bind(("127.0.0.1", port)).expect("the port is free");
    let from_page = call.clone();
    let page = PAGE.replace("__SELF__", &self_id);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            if reader.read_line(&mut request).is_err() {
                continue;
            }
            let mut length = 0usize;
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                    break;
                }
                if let Some(value) = header.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; length];
            let _ = reader.read_exact(&mut body);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let (kind, answer) = match path {
                "/" => ("text/html; charset=utf-8", page.clone()),
                "/poll" => ("application/json", Value::Array(std::mem::take(&mut *outbox.lock().unwrap())).to_string()),
                "/send" => {
                    if let Ok(payload) = serde_json::from_slice::<Value>(&body) {
                        from_page.handle_rtc("page", payload);
                    }
                    ("application/json", "{}".to_owned())
                }
                "/report" => {
                    if let Ok(report) = serde_json::from_slice::<Value>(&body) {
                        say(json!({ "event": "page", "report": report }));
                    }
                    ("application/json", "{}".to_owned())
                }
                _ => ("text/plain", String::new()),
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{answer}",
                answer.len()
            );
            let _ = reader.get_mut().write_all(response.as_bytes());
        }
    });
    say(json!({ "event": "listening", "port": port }));

    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            Some(event) = events.recv() => match event {
                CallEvent::Peer { state, .. } => say(json!({ "event": "peer", "connected": state == PeerState::Connected })),
                CallEvent::Speaking { id, speaking } if id == "page" => say(json!({ "event": "heard", "speaking": speaking })),
                CallEvent::Report(line) => say(json!({ "event": "report", "line": line })),
                _ => {}
            },
        }
    }
    call.stop();
    say(json!({ "event": "stopped" }));
}
