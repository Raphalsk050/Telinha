//! The capture and stream engine, `telinha.exe`, run as a child process. It speaks one JSON event
//! per line on its output and takes one JSON command per line on its input. Same contract as
//! `desktop/src/telinha-process.js` used.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;

use crate::envelope::normalize_code;
use crate::ports::{PortWindow, random_port_window};
use crate::servers::Relay;

pub const CODE_PREFIX: &str = "TELINHA1.";
const CODE_MAGIC: &str = "VExT";
const STOP_GRACE: Duration = Duration::from_secs(4);
const LIST_TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Where the engine is: the override, then next to the app, then the build of this repository.
pub fn resolve_engine() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("TELINHA_EXE").filter(|path| !path.is_empty()) {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(folder) = exe.parent() {
            candidates.push(folder.join("telinha-engine.exe"));
            candidates.push(folder.join("resources").join("telinha.exe"));
            // client/target/<profile>/ in a checkout of the repository.
            for up in folder.ancestors().skip(1).take(4) {
                candidates.push(up.join("build").join("msvc-release").join("tools").join("telinha").join("Release").join("telinha.exe"));
            }
        }
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

/// One JSON event of a line of the engine's output, or nothing for a line that is only a log.
pub fn parse_event_line(line: &str) -> Option<Value> {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let value: Value = serde_json::from_str(trimmed).ok()?;
    value.get("event")?.as_str()?;
    Some(value)
}

/// Whether a pasted text is a connection code, with or without its prefix.
pub fn looks_like_code(text: &str) -> bool {
    let code = normalize_code(text);
    let body = code.strip_prefix(CODE_PREFIX).unwrap_or(&code);
    body.len() > 16
        && body.starts_with(CODE_MAGIC)
        && body.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Monitor,
    Window,
    Device,
}

impl TargetKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Monitor => "monitor",
            Self::Window => "window",
            Self::Device => "device",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareAudio {
    /// Everything the computer plays, leaving out the process given, which is this app.
    System { exclude_pid: Option<u32> },
    /// Only what one program plays.
    Process { pid: u32 },
    /// The sound input of a capture card.
    Device { id: String },
    None,
}

#[derive(Debug, Clone)]
pub struct ShareRequest {
    pub kind: TargetKind,
    /// The stable handle of the target. Preferred over the index, which shifts as windows open.
    pub handle: Option<u64>,
    pub index: Option<u32>,
    pub audio: ShareAudio,
    pub fps: Option<u32>,
    /// One sender for many viewers, as in a call.
    pub multi: bool,
    pub relay: Option<Relay>,
    pub ports: PortWindow,
}

fn is_audio_device_id(id: &str) -> bool {
    (1..=127).contains(&id.len())
        && id.chars().all(|character| character.is_ascii_alphanumeric() || "_{}.-".contains(character))
}

pub fn build_share_args(request: &ShareRequest) -> Result<Vec<String>, String> {
    let kind = request.kind.key();
    let mut args = vec!["send".to_owned(), "--json".to_owned()];
    match (request.handle.filter(|handle| *handle > 0), request.index) {
        (Some(handle), _) => args.extend([format!("--{kind}-handle"), handle.to_string()]),
        (None, Some(index)) => args.extend([format!("--{kind}"), index.to_string()]),
        (None, None) => return Err("escolha uma tela, um programa ou uma placa de captura para compartilhar".into()),
    }
    if request.multi {
        args.push("--multi".into());
    }
    args.push("--audio".into());
    match &request.audio {
        ShareAudio::System { exclude_pid } => {
            args.push("system".into());
            if let Some(pid) = exclude_pid.filter(|pid| *pid > 0) {
                args.extend(["--audio-exclude-pid".to_owned(), pid.to_string()]);
            }
        }
        ShareAudio::Process { pid } => {
            if *pid == 0 {
                return Err("o som por programa precisa do processo do programa".into());
            }
            args.extend(["process".to_owned(), "--audio-pid".to_owned(), pid.to_string()]);
        }
        ShareAudio::Device { id } => {
            if !is_audio_device_id(id) {
                return Err("escolha a entrada de som da placa de captura".into());
            }
            args.extend(["device".to_owned(), "--audio-device".to_owned(), id.clone()]);
        }
        ShareAudio::None => args.push("none".into()),
    }
    if let Some(fps) = request.fps {
        args.extend(["--fps".to_owned(), fps.to_string()]);
    }
    if let Some(relay) = &request.relay {
        for url in relay.urls.iter().filter(|url| url.starts_with("turn")) {
            args.extend(["--turn".to_owned(), url.clone()]);
            if !relay.username.is_empty() {
                args.extend(["--turn-user".to_owned(), relay.username.clone()]);
            }
            if !relay.credential.is_empty() {
                args.extend(["--turn-pass".to_owned(), relay.credential.clone()]);
            }
        }
    }
    args.extend(port_args(request.ports));
    Ok(args)
}

fn port_args(ports: PortWindow) -> [String; 4] {
    ["--port-min".to_owned(), ports.min.to_string(), "--port-max".to_owned(), ports.max.to_string()]
}

pub fn build_watch_args(fullscreen: bool, audio: bool, title: &str, ports: PortWindow) -> Vec<String> {
    let mut args = vec!["recv".to_owned(), "--json".to_owned()];
    if fullscreen {
        args.push("--fullscreen".into());
    }
    if !title.is_empty() {
        args.extend(["--title".to_owned(), title.to_owned()]);
    }
    if !audio {
        args.push("--no-audio".into());
    }
    args.extend(port_args(ports));
    args
}

/// A fresh window of ports for one stream.
pub fn stream_ports() -> PortWindow {
    random_port_window(None)
}

/// The engine takes the window title in plain ASCII.
pub fn ascii_title(text: &str) -> String {
    let plain: String = text
        .chars()
        .filter_map(|character| match character {
            'á' | 'à' | 'â' | 'ã' | 'ä' => Some('a'),
            'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => Some('A'),
            'é' | 'è' | 'ê' | 'ë' => Some('e'),
            'É' | 'È' | 'Ê' | 'Ë' => Some('E'),
            'í' | 'ì' | 'î' | 'ï' => Some('i'),
            'Í' | 'Ì' | 'Î' | 'Ï' => Some('I'),
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => Some('o'),
            'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => Some('O'),
            'ú' | 'ù' | 'û' | 'ü' => Some('u'),
            'Ú' | 'Ù' | 'Û' | 'Ü' => Some('U'),
            'ç' => Some('c'),
            'Ç' => Some('C'),
            'ñ' => Some('n'),
            'Ñ' => Some('N'),
            ' '..='~' => Some(character),
            _ => None,
        })
        .collect();
    plain.trim().chars().take(80).collect()
}

/// What the engine can share right now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Targets {
    pub monitors: Vec<Value>,
    pub windows: Vec<Value>,
    pub devices: Vec<Value>,
    pub audio_inputs: Vec<Value>,
}

fn hidden(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

pub async fn list_targets(exe: &Path) -> Result<Targets, String> {
    let mut command = Command::new(exe);
    hidden(command.args(["list", "--json"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true));
    let output = tokio::time::timeout(LIST_TIMEOUT, command.output())
        .await
        .map_err(|_| "o telinha demorou demais para listar as telas".to_owned())?
        .map_err(|error| format!("não consegui abrir o motor de captura: {error}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let event = text
        .lines()
        .filter_map(parse_event_line)
        .find(|event| event["event"] == "targets")
        .ok_or_else(|| format!("o telinha não listou as telas (código {:?})", output.status.code()))?;
    let list = |name: &str| event.get(name).and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(Targets { monitors: list("monitors"), windows: list("windows"), devices: list("devices"), audio_inputs: list("audio_inputs") })
}

#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    /// One JSON event, as the engine wrote it.
    Event(Value),
    /// A line of plain log.
    Log(String),
    /// The process ended. `stopped` tells an end that was asked for from one that was not.
    Exit { code: Option<i32>, stopped: bool },
}

/// A running engine. Dropping it does not stop the process; `stop` does.
pub struct EngineSession {
    input: Arc<Mutex<Option<ChildStdin>>>,
    stopping: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    kill: mpsc::UnboundedSender<()>,
}

impl EngineSession {
    /// Starts the engine. Must be called from inside a tokio runtime.
    pub fn start(exe: &Path, args: &[String]) -> Result<(Self, mpsc::UnboundedReceiver<EngineEvent>), String> {
        let mut command = Command::new(exe);
        hidden(command.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()));
        let mut child = command.spawn().map_err(|error| format!("não consegui abrir o motor de captura: {error}"))?;
        let (events, receiver) = mpsc::unbounded_channel();
        let input = Arc::new(Mutex::new(child.stdin.take()));
        let stopping = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));
        let (kill, mut killed) = mpsc::unbounded_channel::<()>();

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let lines = events.clone();
        tokio::spawn(async move {
            let Some(stdout) = stdout else { return };
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let event = match parse_event_line(&line) {
                    Some(event) => EngineEvent::Event(event),
                    None if line.trim().is_empty() => continue,
                    None => EngineEvent::Log(line),
                };
                let _ = lines.send(event);
            }
        });
        let logs = events.clone();
        tokio::spawn(async move {
            let Some(stderr) = stderr else { return };
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if !line.trim().is_empty() {
                    let _ = logs.send(EngineEvent::Log(line));
                }
            }
        });
        let (stopping_flag, exited_flag) = (Arc::clone(&stopping), Arc::clone(&exited));
        tokio::spawn(async move {
            let status = tokio::select! {
                status = child.wait() => status.ok(),
                _ = killed.recv() => {
                    let _ = child.kill().await;
                    child.wait().await.ok()
                }
            };
            exited_flag.store(true, Ordering::SeqCst);
            // Lets the last lines of output arrive before the exit.
            tokio::time::sleep(Duration::from_millis(60)).await;
            let _ = events.send(EngineEvent::Exit {
                code: status.and_then(|status| status.code()),
                stopped: stopping_flag.load(Ordering::SeqCst),
            });
        });
        Ok((Self { input, stopping, exited, kill }, receiver))
    }

    pub fn exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    fn write_line(&self, line: String) -> bool {
        if self.exited() || self.stopping.load(Ordering::SeqCst) {
            return false;
        }
        let input = Arc::clone(&self.input);
        tokio::spawn(async move {
            let taken = input.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
            if let Some(mut stdin) = taken {
                let written = stdin.write_all(line.as_bytes()).await.is_ok() && stdin.flush().await.is_ok();
                if written {
                    *input.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(stdin);
                }
            }
        });
        true
    }

    /// Hands a connection code to the engine. False when the text is not a code.
    pub fn submit_code(&self, text: &str) -> bool {
        looks_like_code(text) && self.write_line(format!("{}\n", normalize_code(text)))
    }

    pub fn send_command(&self, command: &Value) -> bool {
        self.write_line(format!("{command}\n"))
    }

    /// Asks the engine to end, and ends it by force if it does not within a few seconds.
    pub fn stop(&self) {
        if self.exited() || self.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        let input = Arc::clone(&self.input);
        let exited = Arc::clone(&self.exited);
        let kill = self.kill.clone();
        tokio::spawn(async move {
            let taken = input.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
            if let Some(mut stdin) = taken {
                let _ = stdin.write_all(b"stop\n").await;
                let _ = stdin.shutdown().await;
            }
            tokio::time::sleep(STOP_GRACE).await;
            if !exited.load(Ordering::SeqCst) {
                let _ = kill.send(());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const WINDOW: PortWindow = PortWindow { min: 51200, max: 51219 };
    const PORTS: [&str; 4] = ["--port-min", "51200", "--port-max", "51219"];

    fn request(kind: TargetKind, handle: Option<u64>, index: Option<u32>, audio: ShareAudio) -> ShareRequest {
        ShareRequest { kind, handle, index, audio, fps: None, multi: false, relay: None, ports: WINDOW }
    }

    fn expect(args: &[&str]) -> Vec<String> {
        args.iter().chain(PORTS.iter()).map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn one_json_event_is_read_per_line_and_everything_else_is_log() {
        assert_eq!(parse_event_line(r#"{"event":"state","state":"Connected"}"#), Some(json!({ "event": "state", "state": "Connected" })));
        assert_eq!(parse_event_line("  reservando memoria"), None);
        assert_eq!(parse_event_line(r#"{"state":"Connected"}"#), None);
        assert_eq!(parse_event_line("{quebrado"), None);
    }

    #[test]
    fn a_code_is_recognized_with_or_without_prefix_and_whitespace() {
        let sample = format!("{CODE_PREFIX}VExTMQEAAgAxCwAAdj0wDQpvPS0gMjYw");
        assert!(looks_like_code(&sample));
        assert!(looks_like_code(&format!("  {}\n{}  ", &sample[..20], &sample[20..])));
        assert!(looks_like_code(&sample[CODE_PREFIX.len()..]));
        assert!(!looks_like_code("ola, tudo bem?"));
        assert!(!looks_like_code(&format!("{CODE_PREFIX}VExT")));
        assert!(!looks_like_code(&format!("{sample}!")));
    }

    #[test]
    fn share_arguments_follow_the_target_and_the_sound_chosen() {
        let monitor = request(TargetKind::Monitor, None, Some(1), ShareAudio::None);
        assert_eq!(build_share_args(&monitor).unwrap(), expect(&["send", "--json", "--monitor", "1", "--audio", "none"]));
        let window = request(TargetKind::Window, None, Some(3), ShareAudio::Process { pid: 4242 });
        assert_eq!(
            build_share_args(&window).unwrap(),
            expect(&["send", "--json", "--window", "3", "--audio", "process", "--audio-pid", "4242"])
        );
        assert!(build_share_args(&request(TargetKind::Monitor, None, None, ShareAudio::None)).is_err());
        assert!(build_share_args(&request(TargetKind::Window, None, Some(0), ShareAudio::Process { pid: 0 })).is_err());
    }

    #[test]
    fn the_stable_handle_is_preferred_over_the_list_index() {
        let window = request(TargetKind::Window, Some(0x1a2b), Some(7), ShareAudio::System { exclude_pid: None });
        assert_eq!(build_share_args(&window).unwrap(), expect(&["send", "--json", "--window-handle", "6699", "--audio", "system"]));
        let monitor = request(TargetKind::Monitor, Some(65537), None, ShareAudio::None);
        assert_eq!(build_share_args(&monitor).unwrap(), expect(&["send", "--json", "--monitor-handle", "65537", "--audio", "none"]));
        assert!(build_share_args(&request(TargetKind::Monitor, Some(0), None, ShareAudio::None)).is_err());
    }

    #[test]
    fn a_call_stream_serves_many_and_leaves_this_app_out_of_the_sound() {
        let mut live = request(TargetKind::Monitor, Some(65537), None, ShareAudio::System { exclude_pid: Some(777) });
        live.multi = true;
        assert_eq!(
            build_share_args(&live).unwrap(),
            expect(&["send", "--json", "--monitor-handle", "65537", "--multi", "--audio", "system", "--audio-exclude-pid", "777"])
        );
    }

    #[test]
    fn watch_arguments_carry_only_what_was_asked() {
        assert_eq!(build_watch_args(false, true, "", WINDOW), expect(&["recv", "--json"]));
        assert_eq!(build_watch_args(true, false, "", WINDOW), expect(&["recv", "--json", "--fullscreen", "--no-audio"]));
        assert_eq!(build_watch_args(false, true, "Telinha - Keep", WINDOW)[2..4], ["--title", "Telinha - Keep"]);
        let ports = stream_ports();
        assert!(ports.min > 50100 && ports.max == ports.min + 19 && ports.max <= 59999);
    }

    #[test]
    fn the_relay_of_the_server_goes_to_the_sender() {
        let relay = Relay {
            urls: vec!["turn:relay.example:3478".into(), "turn:10.0.0.2:3478".into()],
            username: "u".into(),
            credential: "p".into(),
        };
        let mut share = request(TargetKind::Monitor, None, Some(0), ShareAudio::None);
        share.relay = Some(relay);
        let args = build_share_args(&share).unwrap();
        let at = args.iter().position(|arg| arg == "--turn").unwrap();
        assert_eq!(
            args[at..at + 12],
            ["--turn", "turn:relay.example:3478", "--turn-user", "u", "--turn-pass", "p", "--turn", "turn:10.0.0.2:3478", "--turn-user", "u", "--turn-pass", "p"]
        );
    }

    #[test]
    fn a_window_title_goes_to_the_engine_in_plain_ascii() {
        assert_eq!(ascii_title("Telinha - João Ação 😀"), "Telinha - Joao Acao");
        assert_eq!(ascii_title(&"x".repeat(200)).len(), 80);
    }

    /// Needs the engine built: lists what it can share, as the picker will.
    #[tokio::test]
    async fn the_engine_lists_what_it_can_share() {
        let Some(exe) = resolve_engine() else {
            eprintln!("engine not built, skipping");
            return;
        };
        let targets = list_targets(&exe).await.unwrap();
        assert!(!targets.monitors.is_empty());
        assert!(targets.monitors[0].get("handle").is_some());
    }
}
