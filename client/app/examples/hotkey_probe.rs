//! Checks the keyboard shortcuts with a real window of this process in front, as the app is
//! when the person records a shortcut: opens a window, waits for it to be in front, sends F24
//! through Windows (a key no keyboard has, which types nothing) and prints what the hook heard.
//!
//! `cargo run -p telinha --example hotkey_probe`

use std::time::{Duration, Instant};

use telinha::shortcuts::{Action, Hotkeys, Shortcut};

struct Probe {
    hotkeys: Option<Hotkeys>,
    started: Instant,
    sent: bool,
    heard: Vec<String>,
}

fn send_f24() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY};
    let key = |up: bool| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0x87), wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, time: 0, dwExtraInfo: 0 } } };
    // SAFETY: two plain keyboard inputs of the size Windows expects.
    let sent = unsafe { SendInput(&[key(false), key(true)], std::mem::size_of::<INPUT>() as i32) };
    println!("SendInput sent {sent} of 2");
}

fn foreground_is_mine() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let mut pid = 0;
    // SAFETY: plain lookups.
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    pid == std::process::id()
}

impl eframe::App for Probe {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        if self.hotkeys.is_none() {
            self.hotkeys = Hotkeys::start(context, &[Shortcut { action: Action::ShowWindow, keys: vec![0x87], enabled: true }]);
            println!("hook in place: {}", self.hotkeys.is_some());
        }
        let hotkeys = self.hotkeys.as_ref().expect("hooks");
        while let Some(event) = hotkeys.next_event() {
            println!("heard {event:?} (window in front: {})", foreground_is_mine());
            self.heard.push(format!("{event:?}"));
        }
        let age = self.started.elapsed();
        if std::env::var_os("PROBE_MINIMIZE").is_some() && age > Duration::from_millis(800) && age < Duration::from_millis(900) {
            context.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        if !self.sent && age > Duration::from_millis(1500) {
            self.sent = true;
            println!("window in front before sending: {}", foreground_is_mine());
            hotkeys.record(Some(0));
            send_f24();
            std::thread::sleep(Duration::from_millis(200));
            send_f24();
        }
        if age > Duration::from_secs(4) {
            println!("result: {:?}", self.heard);
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        context.request_repaint_after(Duration::from_millis(50));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Keys reach the window too, as in the app.
        let keys = ui.input(|input| input.events.iter().filter(|event| matches!(event, egui::Event::Key { .. })).count());
        if keys > 0 {
            println!("the window got {keys} key events");
        }
        ui.label(format!("{:?}", self.heard));
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 120.0]).with_active(true), ..Default::default() };
    eframe::run_native("hotkey probe", options, Box::new(|_| Ok(Box::new(Probe { hotkeys: None, started: Instant::now(), sent: false, heard: Vec::new() }))))
}
