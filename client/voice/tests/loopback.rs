//! Two calls in one process, wired to each other, with a tone in place of the microphones: both
//! must connect and each must hear the other speak.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use telinha_voice::{Call, CallEvent, Participant, PeerState, Sound, default_ice_servers};
use tokio::sync::mpsc;

/// Processor time of this process so far, user and kernel, in milliseconds.
fn process_ms() -> f64 {
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let (mut created, mut ended, mut kernel, mut user) = Default::default();
    // SAFETY: the pseudo handle of this process is always valid.
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), &mut created, &mut ended, &mut kernel, &mut user);
    }
    let ticks = |time: windows::Win32::Foundation::FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    (ticks(kernel) + ticks(user)) as f64 / 10_000.0
}

fn participant(id: &str) -> Participant {
    Participant { id: id.to_owned(), answers_repeats: true, name: id.to_owned() }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_people_connect_and_hear_each_other() {
    let (to_bia, mut for_bia) = mpsc::unbounded_channel::<Value>();
    let (to_ana, mut for_ana) = mpsc::unbounded_channel::<Value>();
    let kinds: Arc<Mutex<Vec<String>>> = Arc::default();

    let seen = Arc::clone(&kinds);
    let (ana, mut ana_events) = Call::start("ana", default_ice_servers(), Sound::Tone, true, false, move |to, payload| {
        assert_eq!(to, "bia");
        seen.lock().unwrap().push(format!("ana:{}", payload["kind"].as_str().unwrap()));
        let _ = to_bia.send(payload);
    });
    let seen = Arc::clone(&kinds);
    let (bia, mut bia_events) = Call::start("bia", default_ice_servers(), Sound::Tone, true, false, move |to, payload| {
        assert_eq!(to, "ana");
        seen.lock().unwrap().push(format!("bia:{}", payload["kind"].as_str().unwrap()));
        let _ = to_ana.send(payload);
    });
    ana.set_participants(vec![participant("bia")]);
    bia.set_participants(vec![participant("ana")]);

    let (mut ana_connected, mut bia_connected, mut ana_heard, mut bia_heard) = (false, false, false, false);
    let mut ana_ping = None;
    let deadline = tokio::time::sleep(Duration::from_secs(30));
    tokio::pin!(deadline);
    while !(ana_connected && bia_connected && ana_heard && bia_heard && ana_ping.is_some()) {
        tokio::select! {
            Some(payload) = for_bia.recv() => bia.handle_rtc("ana", payload),
            Some(payload) = for_ana.recv() => ana.handle_rtc("bia", payload),
            Some(event) = ana_events.recv() => match event {
                CallEvent::Peer { state: PeerState::Connected, .. } => ana_connected = true,
                CallEvent::Speaking { id, speaking: true } if id == "bia" => ana_heard = true,
                CallEvent::Stats(stats) if stats.rtt.is_some() => ana_ping = Some(stats),
                _ => {}
            },
            Some(event) = bia_events.recv() => match event {
                CallEvent::Peer { state: PeerState::Connected, .. } => bia_connected = true,
                CallEvent::Speaking { id, speaking: true } if id == "ana" => bia_heard = true,
                _ => {}
            },
            _ = &mut deadline => break,
        }
    }
    let kinds = kinds.lock().unwrap().clone();
    println!("messages: {kinds:?}");
    assert!(ana_connected && bia_connected, "both sides connect: ana {ana_connected}, bia {bia_connected}");
    assert!(ana_heard && bia_heard, "both sides hear the other: ana {ana_heard}, bia {bia_heard}");
    // The call measures its ping, for the balloon and the bars of "Voz conectada".
    let ping = ana_ping.expect("the call reports a ping");
    println!("ping: {ping:?}");
    assert_eq!((ping.peers, ping.connected), (1, 1));
    // The one whose id sorts first offers, the other answers.
    assert_eq!(kinds.iter().find(|kind| kind.ends_with("offer")).map(String::as_str), Some("ana:offer"));
    assert!(kinds.iter().any(|kind| kind == "bia:answer"));
    assert!(!kinds.iter().any(|kind| kind == "bia:offer"));
    ana.stop();
    bia.stop();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// A file goes through a real data channel between two connections of this process: who
/// receives makes the offer, who has the file answers and pours it in, and every byte arrives
/// in order.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_crosses_a_data_channel_whole_and_in_order() {
    use telinha_core::hub::{FileLink, PIECE_BYTES, Receiving, Serving};
    use telinha_voice::filelink::WebRtcLink;

    // No outside server: the two ends find each other by the addresses of this computer.
    let receiver = WebRtcLink::new(Vec::new());
    let holder = WebRtcLink::new(Vec::new());
    let Receiving { offer, answer, mut pieces } = receiver.receive().await.expect("the receiving end opens");
    assert!(offer.contains("a=candidate:") && offer.contains("webrtc-datachannel"), "the offer carries its candidates:\n{offer}");
    let Serving { answer: sdp, pieces: sink } = holder.serve(offer).await.expect("the holding end answers");
    assert!(sdp.contains("a=candidate:"));
    answer.send(sdp).unwrap();

    let data: Vec<u8> = (0..6 * 1024 * 1024u32).map(|index| (index.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
    let source = data.clone();
    let pour = tokio::spawn(async move {
        for piece in source.chunks(PIECE_BYTES) {
            sink.send(piece.to_vec()).await.expect("the channel takes the piece");
        }
    });
    let mut arrived = Vec::with_capacity(data.len());
    let started = std::time::Instant::now();
    while arrived.len() < data.len() {
        let piece = tokio::time::timeout(Duration::from_secs(30), pieces.recv()).await.expect("the file keeps coming").expect("the channel stays open until the end");
        arrived.extend_from_slice(&piece);
    }
    pour.await.unwrap();
    println!("6 MB crossed the data channel in {:.2} s", started.elapsed().as_secs_f32());
    assert!(arrived == data, "what arrived is what was sent");
    // Dropping the receiving end closes the connection, which is what ends the other side.
    drop(pieces);
}
/// Run by hand: `cargo test -p telinha-voice --test loopback engine_stream -- --ignored --nocapture`.
/// Asks the real capture engine for a stream the way a browser would, with descriptions
/// instead of codes, and tells whether this library can decode what the engine sends. On
/// 2026-11-14 it could not: the engine offers only H264 and the library refused the video line.
/// Since the vendored webrtc-sys decodes H264 through Media Foundation, frames arrive. Needs
/// the engine built and a monitor.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn engine_stream_is_received_and_decoded_here() {
    use futures_util::StreamExt;
    use libwebrtc::media_stream_track::MediaStreamTrack;
    use libwebrtc::peer_connection::AnswerOptions;
    use libwebrtc::peer_connection_factory::{PeerConnectionFactory, RtcConfiguration};
    use libwebrtc::session_description::{SdpType, SessionDescription};
    use libwebrtc::video_stream::native::NativeVideoStream;
    use telinha_core::engine::{EngineEvent, EngineSession, ShareAudio, ShareRequest, TargetKind, build_share_args, list_targets, resolve_engine, stream_ports};

    let exe = resolve_engine().expect("the engine is built");
    let targets = list_targets(&exe).await.unwrap();
    let monitor = targets.monitors.first().and_then(|monitor| monitor.get("handle")).and_then(Value::as_u64).expect("a monitor");
    let request = ShareRequest { kind: TargetKind::Monitor, handle: Some(monitor), index: None, audio: ShareAudio::None, fps: None, multi: true, relay: None, ports: stream_ports() };
    let (engine, mut events) = EngineSession::start(&exe, &build_share_args(&request).unwrap()).unwrap();

    let factory = PeerConnectionFactory::default();
    let connection = factory.create_peer_connection(RtcConfiguration::default()).unwrap();
    let (tracks, mut video) = mpsc::unbounded_channel();
    connection.on_track(Some(Box::new(move |event| {
        if let MediaStreamTrack::Video(track) = event.track {
            let _ = tracks.send(track);
        }
    })));
    connection.on_connection_state_change(Some(Box::new(|state| println!("connection here: {state:?}"))));
    let candidates: Arc<Mutex<Vec<String>>> = Arc::default();
    let found = Arc::clone(&candidates);
    connection.on_ice_candidate(Some(Box::new(move |candidate| found.lock().unwrap().push(candidate.candidate()))));

    let deadline = tokio::time::sleep(Duration::from_secs(40));
    tokio::pin!(deadline);
    let mut frames = 0u32;
    let mut stream: Option<NativeVideoStream> = None;
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(EngineEvent::Event(event)) => match event["event"].as_str().unwrap_or("") {
                    "ready" => {
                        println!("engine ready: {}x{}", event["width"], event["height"]);
                        assert!(engine.send_command(&serde_json::json!({ "command": "add_peer", "peer": "probe", "format": "sdp" })));
                    }
                    "offer" => {
                        let sdp = event["sdp"].as_str().expect("the offer has a description").to_owned();
                        let codecs: Vec<&str> = sdp.lines().filter(|line| line.starts_with("a=rtpmap:")).collect();
                        println!("the engine offers: {codecs:?}");
                        connection.set_remote_description(SessionDescription::parse(&sdp, SdpType::Offer).unwrap()).await.unwrap();
                        let answer = connection.create_answer(AnswerOptions::default()).await.unwrap();
                        let mut text = answer.to_string();
                        let lines: Vec<&str> = text.lines().filter(|line| line.starts_with("m=") || line.starts_with("a=rtpmap:") || line.starts_with("a=fmtp:")).collect();
                        println!("the answer says: {lines:?}");
                        // A video line with port zero is a refusal: nothing here can decode what was offered.
                        if text.lines().any(|line| line.starts_with("m=video 0 ")) {
                            println!("VERDICT: the engine sends only H264 and this WebRTC library has no H264 decoder on Windows, so the picture cannot be shown inside the window through it");
                            engine.stop();
                            connection.close();
                            return;
                        }
                        connection.set_local_description(answer).await.unwrap();
                        tokio::time::sleep(Duration::from_millis(1500)).await;
                        for candidate in candidates.lock().unwrap().iter() {
                            text.push_str(&format!("a={}\r\n", candidate.trim()));
                        }
                        assert!(engine.send_command(&serde_json::json!({ "command": "peer_sdp_answer", "peer": "probe", "sdp": text })));
                    }
                    "error" | "peer_failed" | "command_failed" => panic!("the engine complained: {event}"),
                    "stats" => {}
                    other => println!("engine: {other} {}", event.get("state").or(event.get("peer")).unwrap_or(&Value::Null)),
                },
                Some(EngineEvent::Exit { code, .. }) => panic!("the engine ended with {code:?}"),
                Some(EngineEvent::Log(_)) => {}
                None => break,
            },
            track = video.recv() => {
                println!("a video track arrived");
                stream = track.map(|track| NativeVideoStream::new(track));
            }
            frame = async { stream.as_mut().unwrap().next().await }, if stream.is_some() => {
                let Some(frame) = frame else { break };
                frames += 1;
                if frames == 1 || frames % 30 == 0 {
                    let buffer = frame.buffer.as_ref();
                    println!("frame {frames}: {}x{} as {:?}", buffer.width(), buffer.height(), buffer.buffer_type());
                }
                if frames >= 90 {
                    break;
                }
            }
            () = &mut deadline => break,
        }
    }
    engine.stop();
    connection.close();
    assert!(frames >= 90, "only {frames} frames were decoded");
}
/// Frames fed to the camera of one side reach the other side as pictures for the window, over
/// the video line that every call opens.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn camera_frames_cross_the_call() {
    use telinha_voice::video::{Nv12, send_nv12};

    let (to_bia, mut for_bia) = mpsc::unbounded_channel::<Value>();
    let (to_ana, mut for_ana) = mpsc::unbounded_channel::<Value>();
    let (ana, _ana_events) = Call::start("ana", default_ice_servers(), Sound::Tone, false, true, move |_, payload| {
        let _ = to_bia.send(payload);
    });
    let (bia, _bia_events) = Call::start("bia", default_ice_servers(), Sound::Tone, false, true, move |_, payload| {
        let _ = to_ana.send(payload);
    });
    ana.set_participants(vec![participant("bia")]);
    bia.set_participants(vec![participant("ana")]);

    // A gray picture with a bright square, 640×360, as a camera would give it.
    let (width, height) = (640u32, 360u32);
    let mut y = vec![60u8; (width * height) as usize];
    for row in 100..200 {
        for column in 200..300 {
            y[row * width as usize + column] = 220;
        }
    }
    let uv = vec![128u8; (width * height / 2) as usize];
    let camera = ana.camera();
    let feeder = tokio::spawn(async move {
        let mut time = 0i64;
        loop {
            send_nv12(&camera, &Nv12 { width, height, y: &y, stride_y: width, uv: &uv, stride_uv: width }, time);
            time += 33_333;
            tokio::time::sleep(Duration::from_millis(33)).await;
        }
    });

    let deadline = tokio::time::sleep(Duration::from_secs(30));
    tokio::pin!(deadline);
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let picture = loop {
        tokio::select! {
            Some(payload) = for_bia.recv() => bia.handle_rtc("ana", payload),
            Some(payload) = for_ana.recv() => ana.handle_rtc("bia", payload),
            _ = tick.tick() => {
                if let Some((_, picture)) = bia.video_of("ana").and_then(|store| store.latest()) {
                    break picture;
                }
            }
            _ = &mut deadline => panic!("no picture of the camera arrived"),
        }
    };
    feeder.abort();
    println!("bia sees ana's camera at {}x{}", picture.width, picture.height);
    assert_eq!((picture.width, picture.height), (width, height));
    // The bright square is still there, at its place.
    let at = |x: u32, y: u32| picture.rgba[((y * picture.width + x) * 4) as usize];
    assert!(at(250, 150) > 180 && at(50, 50) < 110, "square {} background {}", at(250, 150), at(50, 50));
    ana.stop();
    bia.stop();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// Run by hand: `cargo test -p telinha-voice --test loopback own_screen -- --ignored --nocapture`.
/// The person sharing sees their own screen inside the window: the hub asks the real engine for
/// one more viewer that answers with descriptions, and the call receives it as the window would.
/// Needs the engine built and a monitor. Nothing reaches the speakers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn own_screen_shows_inside_the_window() {
    use telinha_core::engine::{ShareAudio, TargetKind, list_targets, resolve_engine};
    use telinha_core::hub::{Hub, HubEvent, SELF_VIEWER, ShareChoice};
    use telinha_core::servers::ChannelKind;

    let folder = std::env::temp_dir().join(format!("telinha-selfview-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let (hub, mut events) = Hub::open(&folder, telinha_core::signaling::default_brokers());
    let server = hub.create_server("Teste");
    let room = server.channels.iter().find(|channel| channel.kind == ChannelKind::Voice).unwrap().id.clone();
    hub.join_voice(&server.id, &room, false, false, true).unwrap();
    let (call, mut call_events) = Call::start(hub.instance_id(), default_ice_servers(), Sound::Tone, false, true, |_, _| {});

    let exe = resolve_engine().expect("the engine is built");
    let targets = list_targets(&exe).await.unwrap();
    let handle = targets.monitors.first().and_then(|monitor| monitor.get("handle")).and_then(Value::as_u64).expect("a monitor");
    hub.go_live(ShareChoice { kind: TargetKind::Monitor, handle, name: "Tela".into(), audio: ShareAudio::None, fps: None }).unwrap();

    let deadline = tokio::time::sleep(Duration::from_secs(40));
    tokio::pin!(deadline);
    let mut look = tokio::time::interval(Duration::from_millis(200));
    let mut pictures = 0;
    loop {
        tokio::select! {
            Some(event) = events.recv() => if let HubEvent::EmbeddedOffer { sharer_id, sdp } = event {
                println!("offer for {sharer_id}");
                call.open_stream(&sharer_id, &sdp);
            },
            Some(event) = call_events.recv() => match event {
                CallEvent::StreamAnswer { sharer_id, sdp } => {
                    println!("answer for {sharer_id}");
                    assert!(hub.answer_embedded(&sharer_id, &sdp));
                }
                CallEvent::StreamState { sharer_id, live } => println!("{sharer_id} live: {live}"),
                _ => {}
            },
            _ = look.tick() => {
                if let Some((_, picture)) = call.stream_video(SELF_VIEWER).and_then(|store| store.latest()) {
                    pictures += 1;
                    if pictures == 1 {
                        println!("first picture: {}x{}", picture.width, picture.height);
                    }
                    if pictures >= 10 {
                        // With TELINHA_COLOR_DUMP set, the last frame is written there as raw
                        // RGBA, to compare its colors with the screen.
                        if let Some(path) = std::env::var_os("TELINHA_COLOR_DUMP") {
                            std::fs::write(&path, &picture.rgba).unwrap();
                            println!("dumped {}x{}", picture.width, picture.height);
                        }
                        break;
                    }
                }
            }
            () = &mut deadline => break,
        }
    }
    hub.stop_live();
    call.stop();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = std::fs::remove_dir_all(&folder);
    assert!(pictures >= 10, "only {pictures} pictures of the own screen arrived");
}

/// Run by hand: `cargo test -p telinha-voice --test loopback screen_watched -- --ignored --nocapture`.
/// Someone watches a screen inside the window: two hubs in the same room, wired in memory as
/// the message servers would carry them, the sharer with the real engine and the viewer with
/// the call receiving it. Needs the engine built and a monitor. Nothing reaches the speakers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn screen_watched_inside_the_window() {
    use telinha_core::engine::{ShareAudio, TargetKind, list_targets, resolve_engine};
    use telinha_core::hub::{Hub, HubEvent, IncomingPhase, ShareChoice};
    use telinha_core::servers::ChannelKind;
    use telinha_core::signaling::Broker;

    fn wire(from: &Arc<Hub>, to: &Arc<Hub>) {
        let (sink, mut sent) = mpsc::unbounded_channel::<(String, Value)>();
        from.signaling().tap(sink);
        let to = Arc::clone(to);
        tokio::spawn(async move {
            while let Some((space_id, message)) = sent.recv().await {
                to.signaling().loopback(&space_id, &message);
            }
        });
    }

    let base = std::env::temp_dir().join(format!("telinha-watch-{}", std::process::id()));
    let (one, two) = (base.join("sharer"), base.join("viewer"));
    std::fs::create_dir_all(&one).unwrap();
    std::fs::create_dir_all(&two).unwrap();
    let brokers = || vec![Broker::new("wss://127.0.0.1:1/unused")];
    let (sharer, _sharer_events) = Hub::open(&one, brokers());
    let (viewer, mut viewer_events) = Hub::open(&two, brokers());
    sharer.set_name("Quem mostra");
    viewer.set_name("Quem assiste");
    let server = sharer.create_server("Telas");
    let room = server.channels.iter().find(|channel| channel.kind == ChannelKind::Voice).unwrap().id.clone();
    viewer.join_server(&sharer.invite_code(&server.id).unwrap()).expect("the invite works");
    wire(&sharer, &viewer);
    wire(&viewer, &sharer);
    sharer.signaling().publish_presence(true, None);
    viewer.signaling().publish_presence(true, None);
    for _ in 0..100 {
        if viewer.server(&server.id).is_some_and(|known| known.channels.iter().any(|channel| channel.id == room)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    sharer.join_voice(&server.id, &room, false, false, true).unwrap();
    viewer.join_voice(&server.id, &room, false, false, true).unwrap();
    let (call, mut call_events) = Call::start(viewer.instance_id(), default_ice_servers(), Sound::Tone, false, true, |_, _| {});

    let exe = resolve_engine().expect("the engine is built");
    let targets = list_targets(&exe).await.unwrap();
    let handle = targets.monitors.first().and_then(|monitor| monitor.get("handle")).and_then(Value::as_u64).expect("a monitor");
    sharer.go_live(ShareChoice { kind: TargetKind::Monitor, handle, name: "Tela".into(), audio: ShareAudio::None, fps: None }).unwrap();
    let sharer_id = sharer.instance_id().to_owned();
    let mut asked = false;
    for _ in 0..200 {
        let live = viewer.peers(&server.id).iter().any(|peer| peer.id == sharer_id && peer.voice.as_ref().is_some_and(|voice| voice.live));
        if live {
            viewer.watch(&sharer_id).expect("the screen can be asked for");
            asked = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(asked, "the viewer saw the sharer go live");

    let deadline = tokio::time::sleep(Duration::from_secs(40));
    tokio::pin!(deadline);
    let mut look = tokio::time::interval(Duration::from_millis(200));
    let mut pictures = 0;
    let mut numbers = Vec::new();
    loop {
        tokio::select! {
            Some(event) = viewer_events.recv() => match event {
                HubEvent::EmbeddedOffer { sharer_id, sdp } => {
                    println!("offer from {sharer_id}");
                    call.open_stream(&sharer_id, &sdp);
                }
                HubEvent::IncomingEnded { reason, error, .. } => panic!("the watching ended: {reason:?} {error:?}"),
                _ => {}
            },
            Some(event) = call_events.recv() => match event {
                CallEvent::StreamAnswer { sharer_id, sdp } => assert!(viewer.answer_embedded(&sharer_id, &sdp)),
                CallEvent::StreamState { sharer_id, live } => {
                    println!("{sharer_id} live: {live}");
                    viewer.embedded_state(&sharer_id, live);
                }
                CallEvent::StreamStats { stats, .. } => {
                    println!("stats: {stats:?}");
                    numbers.push(stats);
                }
                _ => {}
            },
            _ = look.tick() => {
                if let Some((_, picture)) = call.stream_video(&sharer_id).and_then(|store| store.latest()) {
                    pictures += 1;
                    if pictures == 1 {
                        println!("first picture: {}x{}", picture.width, picture.height);
                    }
                    if pictures >= 10 && numbers.len() >= 2 {
                        break;
                    }
                }
            }
            () = &mut deadline => break,
        }
    }
    // With TELINHA_MEASURE_SECS set, the processor time of receiving and decoding the screen is
    // measured with no window taking the frames, as when nothing shows the video.
    if let Some(seconds) = std::env::var("TELINHA_MEASURE_SECS").ok().and_then(|value| value.parse::<u64>().ok()) {
        let before = process_ms();
        let start = std::time::Instant::now();
        tokio::time::sleep(Duration::from_secs(seconds)).await;
        let used = process_ms() - before;
        let elapsed = start.elapsed().as_secs_f64();
        let mut last = None;
        while let Ok(event) = call_events.try_recv() {
            if let CallEvent::StreamStats { stats, .. } = event {
                last = Some(stats);
            }
        }
        println!("receiving and decoding: {:.1} % of one core over {elapsed:.1} s; last reading {last:?}", used / (elapsed * 1000.0) * 100.0);
    }
    let phase = viewer.streams().incoming.first().map(|entry| entry.phase);
    // The small picture of the share reaches the room.
    assert!(sharer.publish_stream_thumb("data:image/jpeg;base64,/9j/4AAQSkZJRgABAQ=="));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(viewer.stream_thumb(&sharer_id).is_some(), "the viewer got the picture");
    sharer.stop_live();
    call.stop();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = std::fs::remove_dir_all(&base);
    assert!(pictures >= 10, "only {pictures} pictures of the screen arrived");
    assert_eq!(phase, Some(IncomingPhase::Live));
    let last = numbers.last().expect("the watching reported its numbers");
    assert!(last.size.is_some() && last.rtt.is_some() && last.bitrate.is_some_and(|kbps| kbps > 0.0), "{last:?}");
}
