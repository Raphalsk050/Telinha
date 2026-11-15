//! Measures how much processor the window takes, apart from the rest of the app: a real window
//! with made-up data, a screen of 1920x1080 at 60 frames per second watched inside it, and the
//! time each part used. Nothing connects to anyone and nothing plays on the speakers.
//!
//!   cargo run --release -p telinha --example ui_cost

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use libwebrtc::video_frame::{I420Buffer, VideoBuffer};
use serde_json::{Value, json};
use telinha::app::{Nav, TelinhaApp};
use telinha_core::hub::Hub;
use telinha_core::servers::ChannelKind;
use telinha_core::signaling::default_brokers;
use telinha_core::text::now_ms;
use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId, GetProcessTimes, GetThreadTimes, OpenThread, THREAD_QUERY_LIMITED_INFORMATION};

const KEEP: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const NANDA: &str = "cccccccccccccccccccccccccccccccc";
const WARM: Duration = Duration::from_secs(3);
const MEASURE: Duration = Duration::from_secs(10);

/// What the window does in each part of the measurement.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// The chat, nothing moving.
    Idle,
    /// The call, with the screen of Keep on the stage.
    Stage,
    /// The screen over the whole window.
    Full,
    /// The window minimized while the screen keeps coming.
    Minimized,
    Done,
}

#[derive(Default)]
struct Shared {
    phase: Mutex<Option<Phase>>,
    /// Time inside `TelinhaApp::ui`, in nanoseconds, and passes of it.
    build_ns: AtomicU64,
    passes: AtomicU64,
    ui_thread: Mutex<Option<isize>>,
    producer_thread: Mutex<Option<isize>>,
    feeding: AtomicBool,
    /// Why the window drew again, by place in the code, in the phase measured now.
    causes: Mutex<std::collections::HashMap<String, u64>>,
}

fn ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// Processor time of a thread, user and kernel, in milliseconds.
fn thread_ms(handle: Option<isize>) -> f64 {
    let Some(handle) = handle else { return 0.0 };
    let (mut created, mut ended, mut kernel, mut user) = Default::default();
    // SAFETY: the handle was opened for querying and is never closed while the program runs.
    unsafe {
        if GetThreadTimes(HANDLE(handle as *mut _), &mut created, &mut ended, &mut kernel, &mut user).is_err() {
            return 0.0;
        }
    }
    (ticks(kernel) + ticks(user)) as f64 / 10_000.0
}

fn process_ms() -> f64 {
    let (mut created, mut ended, mut kernel, mut user) = Default::default();
    // SAFETY: the pseudo handle of this process is always valid.
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), &mut created, &mut ended, &mut kernel, &mut user);
    }
    (ticks(kernel) + ticks(user)) as f64 / 10_000.0
}

fn own_thread() -> Option<isize> {
    // SAFETY: opens the calling thread for reading its times; the handle is kept for good.
    unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, GetCurrentThreadId()).ok().map(|handle| handle.0 as isize) }
}

fn presence(from: &str, name: &str, member_id: &str, extra: Value) -> Value {
    let mut message = json!({
        "type": "presence", "online": true, "from": from, "at": now_ms(), "mid": telinha_core::text::new_uuid(),
        "name": name, "memberId": member_id,
    });
    message.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    message
}

struct Probe {
    app: TelinhaApp,
    shared: Arc<Shared>,
    hub: Arc<Hub>,
    runtime: tokio::runtime::Handle,
    server_id: String,
    room: String,
    applied: Option<Phase>,
    /// The screen of Keep waits for the app to see it is in the call.
    stream_due: bool,
}

impl Probe {
    fn apply(&mut self, phase: Phase, context: &egui::Context) {
        match phase {
            Phase::Idle | Phase::Done => {}
            Phase::Stage => {
                let _guard = self.runtime.enter();
                self.hub.join_voice(&self.server_id, &self.room, true, false, false).unwrap();
                self.app.navigate(Nav::Server { server_id: self.server_id.clone(), channel_id: Some(self.room.clone()) });
                self.stream_due = true;
            }
            Phase::Full => self.app.open_viewer("peer-keep", "Keep".to_owned()),
            Phase::Minimized => {
                self.app.close_viewer();
                context.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
    }
}

impl eframe::App for Probe {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut thread = self.shared.ui_thread.lock().unwrap();
        if thread.is_none() {
            *thread = own_thread();
        }
        drop(thread);
        let wanted = *self.shared.phase.lock().unwrap();
        if let Some(phase) = wanted.filter(|phase| Some(*phase) != self.applied) {
            self.applied = Some(phase);
            self.apply(phase, ui.ctx());
        }
        if self.stream_due {
            if let Some(store) = self.app.stream_for_test("peer-keep", "Keep", 2, 2, vec![0; 16]) {
                self.stream_due = false;
                start_producer(store, Arc::clone(&self.shared));
            }
        }
        for cause in ui.ctx().repaint_causes() {
            *self.shared.causes.lock().unwrap().entry(format!("{}:{} {}", cause.file, cause.line, cause.reason)).or_default() += 1;
        }
        let start = Instant::now();
        self.app.ui(ui);
        self.shared.build_ns.fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        self.shared.passes.fetch_add(1, Ordering::Relaxed);
        // The phases change from another thread; this makes sure the window looks again.
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }
}

/// Plays the part of the decoder: a 1920x1080 frame every 16.7 ms into the store of the screen.
fn start_producer(store: Arc<telinha_voice::video::FrameStore>, shared: Arc<Shared>) {
    shared.feeding.store(true, Ordering::Relaxed);
    std::thread::spawn(move || {
        *shared.producer_thread.lock().unwrap() = own_thread();
        let mut frames: Vec<I420Buffer> = (0..4u8)
            .map(|shade| {
                let mut buffer = I420Buffer::new(1920, 1080);
                let (stride_y, _, _) = buffer.strides();
                let (y, u, v) = buffer.data_mut();
                for (index, value) in y.iter_mut().enumerate() {
                    let (column, row) = (index as u32 % stride_y, index as u32 / stride_y);
                    *value = ((column / 8 + row / 8) as u8).wrapping_add(shade * 40);
                }
                u.fill(100 + shade * 10);
                v.fill(150 - shade * 10);
                buffer
            })
            .collect();
        let started = Instant::now();
        let mut count = 0u64;
        while shared.feeding.load(Ordering::Relaxed) {
            let buffer = &mut frames[(count % 4) as usize];
            store.put_buffer(buffer as &dyn VideoBuffer);
            count += 1;
            let next = started + Duration::from_micros(16_667 * count);
            if let Some(wait) = next.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
    });
}

/// Steps through the phases and prints what each one cost.
fn conduct(shared: Arc<Shared>) {
    println!("{:<12} {:>8} {:>10} {:>12} {:>12} {:>10} {:>10}", "fase", "quadros/s", "processo %", "thread UI %", "montar UI %", "conversão %", "outras %");
    for phase in [Phase::Idle, Phase::Stage, Phase::Full, Phase::Minimized] {
        *shared.phase.lock().unwrap() = Some(phase);
        std::thread::sleep(WARM);
        let ui_thread = *shared.ui_thread.lock().unwrap();
        let producer = *shared.producer_thread.lock().unwrap();
        let before = (process_ms(), thread_ms(ui_thread), thread_ms(producer), shared.build_ns.load(Ordering::Relaxed), shared.passes.load(Ordering::Relaxed));
        shared.causes.lock().unwrap().clear();
        let start = Instant::now();
        std::thread::sleep(MEASURE);
        let seconds = start.elapsed().as_secs_f64();
        let after = (process_ms(), thread_ms(ui_thread), thread_ms(producer), shared.build_ns.load(Ordering::Relaxed), shared.passes.load(Ordering::Relaxed));
        // Percent of one processor core.
        let share = |ms: f64| ms / (seconds * 1000.0) * 100.0;
        let process = share(after.0 - before.0);
        let ui = share(after.1 - before.1);
        let convert = share(after.2 - before.2);
        let build = share((after.3 - before.3) as f64 / 1e6);
        let passes = (after.4 - before.4) as f64 / seconds;
        println!(
            "{:<12} {:>8.1} {:>10.1} {:>12.1} {:>12.1} {:>10.1} {:>10.1}",
            format!("{phase:?}"),
            passes,
            process,
            ui,
            build,
            convert,
            (process - ui - convert).max(0.0)
        );
        let mut causes: Vec<(String, u64)> = shared.causes.lock().unwrap().drain().collect();
        causes.sort_by(|a, b| b.1.cmp(&a.1));
        for (cause, count) in causes.into_iter().take(4) {
            println!("    {count:>5}x {cause}");
        }
    }
    *shared.phase.lock().unwrap() = Some(Phase::Done);
    shared.feeding.store(false, Ordering::Relaxed);
    std::process::exit(0);
}

fn main() -> eframe::Result {
    let folder = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().unwrap();
    let guard = runtime.enter();
    let (hub, events) = Hub::open(folder.path(), default_brokers());
    hub.set_name("Raphalsk");
    let server = hub.create_server("Servidor de rapha");
    let room = server.channels.iter().find(|channel| channel.kind == ChannelKind::Voice).unwrap().id.clone();
    let text = server.channels.iter().find(|channel| channel.kind == ChannelKind::Text).unwrap().id.clone();
    let signaling = hub.signaling();
    signaling.loopback(&server.id, &presence("peer-keep", "Keep", KEEP, json!({ "voice": { "roomId": room, "mic": true, "live": true, "liveName": "Jogo", "v": 2 } })));
    signaling.loopback(&server.id, &presence("peer-nanda", "nanda", NANDA, json!({ "voice": { "roomId": room, "mic": false, "v": 2 } })));
    for index in 0..40 {
        let message = json!({
            "type": "chat", "from": "peer-keep", "at": now_ms(), "mid": format!("mid-{index}"),
            "messageId": format!("m{index}"), "channelId": text, "author": "Keep", "memberId": KEEP,
            "sentAt": now_ms() - (40 - index) * 60_000, "text": format!("mensagem número {index} para encher a conversa"),
        });
        signaling.loopback(&server.id, &message);
    }
    runtime.block_on(async { tokio::time::sleep(Duration::from_millis(200)).await });
    drop(guard);

    let shared = Arc::new(Shared::default());
    let conductor = Arc::clone(&shared);
    std::thread::spawn(move || conduct(conductor));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Telinha - medição").with_inner_size([1360.0, 860.0]),
        ..Default::default()
    };
    let handle = runtime.handle().clone();
    let server_id = server.id.clone();
    eframe::run_native(
        "Telinha medicao",
        options,
        Box::new(move |creation| {
            let mut app = TelinhaApp::new(&creation.egui_ctx, Arc::clone(&hub), events, handle.clone());
            // Nothing here connects to anyone or makes a sound.
            app.disable_calls();
            app.navigate(Nav::Server { server_id: server_id.clone(), channel_id: Some(text.clone()) });
            Ok(Box::new(Probe { app, shared, hub, runtime: handle, server_id, room, applied: None, stream_due: false }))
        }),
    )
}
