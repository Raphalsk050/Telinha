//! Keyboard shortcuts that work anywhere in Windows, as Discord's: with the window minimized,
//! behind a game or in another app. A low-level hook of the keyboard and of the mouse, on a
//! thread of its own, sees every key going down and up and never swallows one. Push to talk and
//! push to mute open and close the microphone right there, in the hook; the other actions go to
//! the window, which takes them even while minimized (`App::logic`).
//!
//! Windows does not call that hook while a window of this same app is in front (seen on
//! Windows 11 with `examples/hotkey_probe.rs`: silent with the window in front, heard with it
//! minimized). So a second pair of hooks sits on the thread of the window, and sees the keys the
//! window gets; a key both hooks saw counts once.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender};

use serde_json::{Value, json};

/// What a shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    ToggleMute,
    ToggleDeaf,
    PushToTalk,
    PushToMute,
    ToggleCamera,
    ToggleShare,
    LeaveCall,
    ShowWindow,
}

impl Action {
    /// Every action, in the order of the list to pick from.
    pub const ALL: [Action; 8] = [
        Action::ToggleMute,
        Action::ToggleDeaf,
        Action::PushToTalk,
        Action::PushToMute,
        Action::ToggleCamera,
        Action::ToggleShare,
        Action::LeaveCall,
        Action::ShowWindow,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::ToggleMute => "Alternar silenciar microfone",
            Action::ToggleDeaf => "Alternar ensurdecer",
            Action::PushToTalk => "Pressionar para falar",
            Action::PushToMute => "Pressionar para silenciar",
            Action::ToggleCamera => "Ligar ou desligar a câmera",
            Action::ToggleShare => "Transmitir tela ou parar",
            Action::LeaveCall => "Sair da chamada",
            Action::ShowWindow => "Mostrar o Telinha",
        }
    }

    /// The line under the action that tells what it does, when the name alone does not.
    pub fn note(self) -> &'static str {
        match self {
            Action::PushToTalk => "Com este atalho ligado, o microfone só transmite enquanto as teclas estiverem pressionadas.",
            Action::PushToMute => "O microfone para de transmitir enquanto as teclas estiverem pressionadas.",
            Action::ToggleShare => "Sem transmissão, traz o Telinha para a frente e abre a escolha da tela.",
            _ => "",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Action::ToggleMute => "toggleMute",
            Action::ToggleDeaf => "toggleDeaf",
            Action::PushToTalk => "pushToTalk",
            Action::PushToMute => "pushToMute",
            Action::ToggleCamera => "toggleCamera",
            Action::ToggleShare => "toggleShare",
            Action::LeaveCall => "leaveCall",
            Action::ShowWindow => "showWindow",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }

    /// Whether the action lasts while the keys are held, instead of happening once.
    pub fn held(self) -> bool {
        matches!(self, Action::PushToTalk | Action::PushToMute)
    }
}

/// One shortcut: what it does, its keys by virtual-key code, and whether it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub action: Action,
    pub keys: Vec<u32>,
    pub enabled: bool,
}

impl Shortcut {
    pub fn new(action: Action) -> Self {
        Self { action, keys: Vec::new(), enabled: true }
    }

    pub fn to_json(&self) -> Value {
        json!({ "action": self.action.id(), "keys": self.keys, "enabled": self.enabled })
    }

    pub fn from_json(value: &Value) -> Option<Self> {
        let action = Action::from_id(value.get("action")?.as_str()?)?;
        let keys = value.get("keys").and_then(Value::as_array).map(|keys| keys.iter().filter_map(|key| u32::try_from(key.as_u64()?).ok()).filter(|key| *key > 0 && *key < 0x200).take(MAX_KEYS).collect()).unwrap_or_default();
        Some(Self { action, keys: canonical(keys), enabled: value.get("enabled").and_then(Value::as_bool).unwrap_or(true) })
    }
}

/// The most keys a shortcut takes.
pub const MAX_KEYS: usize = 4;

const SHIFT: u32 = 0x10;
const CTRL: u32 = 0x11;
const ALT: u32 = 0x12;
const WIN: u32 = 0x5B;
/// The Enter of the numeric keypad, which Windows gives the same code as the big one.
const NUM_ENTER: u32 = 0x10D;

/// The key as the shortcuts know it, from what the hook of the keyboard saw. With Num Lock off the
/// numeric keypad sends the codes of the arrows and of Home, End and the like; only the flag of
/// the extended keys tells those apart, and a key of the keypad is always the same key here.
pub fn from_hook(vk: u32, extended: bool) -> u32 {
    match (vk, extended) {
        (0x2D, false) => 0x60,
        (0x23, false) => 0x61,
        (0x28, false) => 0x62,
        (0x22, false) => 0x63,
        (0x25, false) => 0x64,
        (0x0C, false) => 0x65,
        (0x27, false) => 0x66,
        (0x24, false) => 0x67,
        (0x26, false) => 0x68,
        (0x21, false) => 0x69,
        (0x2E, false) => 0x6E,
        (0x0D, true) => NUM_ENTER,
        (other, _) => other,
    }
}
/// The modifiers, in the order they are written.
const MODIFIERS: [u32; 4] = [CTRL, SHIFT, ALT, WIN];

/// The left and the right keys of a modifier count as one, as Discord does.
fn normalize(vk: u32) -> u32 {
    match vk {
        0xA0 | 0xA1 => SHIFT,
        0xA2 | 0xA3 => CTRL,
        0xA4 | 0xA5 => ALT,
        0x5C => WIN,
        other => other,
    }
}

fn is_modifier(vk: u32) -> bool {
    MODIFIERS.contains(&vk)
}

/// The keys in the order they are written: the modifiers first, in their order, then the rest as
/// they were pressed, each once.
pub fn canonical(keys: Vec<u32>) -> Vec<u32> {
    let keys: Vec<u32> = keys.into_iter().map(normalize).collect();
    let mut out: Vec<u32> = MODIFIERS.into_iter().filter(|modifier| keys.contains(modifier)).collect();
    for key in keys {
        if !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

/// "CTRL + SHIFT + M", as Discord writes a shortcut.
pub fn describe(keys: &[u32]) -> String {
    keys.iter().map(|key| key_name(*key)).collect::<Vec<_>>().join(" + ")
}

/// The name of one key, in capitals.
pub fn key_name(vk: u32) -> String {
    let fixed = match vk {
        CTRL => "CTRL",
        SHIFT => "SHIFT",
        ALT => "ALT",
        WIN => "WIN",
        NUM_ENTER => "NUM ENTER",
        0x0C => "NUM 5",
        0x04 => "BOTÃO DO MEIO",
        0x05 => "MOUSE 4",
        0x06 => "MOUSE 5",
        0x08 => "BACKSPACE",
        0x09 => "TAB",
        0x0D => "ENTER",
        0x13 => "PAUSE",
        0x14 => "CAPS LOCK",
        0x1B => "ESC",
        0x20 => "ESPAÇO",
        0x21 => "PAGE UP",
        0x22 => "PAGE DOWN",
        0x23 => "END",
        0x24 => "HOME",
        0x25 => "←",
        0x26 => "↑",
        0x27 => "→",
        0x28 => "↓",
        0x2C => "PRINT SCREEN",
        0x2D => "INSERT",
        0x2E => "DELETE",
        0x5D => "MENU",
        0x6A => "NUM *",
        0x6B => "NUM +",
        0x6D => "NUM -",
        0x6E => "NUM ,",
        0x6F => "NUM /",
        0x90 => "NUM LOCK",
        0x91 => "SCROLL LOCK",
        0xAD => "SEM SOM",
        0xAE => "VOLUME -",
        0xAF => "VOLUME +",
        0xB0 => "PRÓXIMA FAIXA",
        0xB1 => "FAIXA ANTERIOR",
        0xB2 => "PARAR MÍDIA",
        0xB3 => "TOCAR/PAUSAR",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_owned();
    }
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(vk).map(String::from).unwrap_or_default(),
        0x60..=0x69 => format!("NUM {}", vk - 0x60),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => layout_name(vk).unwrap_or_else(|| format!("TECLA {vk}")),
    }
}

/// What the keyboard layout writes with this key, for the keys that change between layouts
/// (Ç, ´, ~ and the like).
#[cfg(windows)]
fn layout_name(vk: u32) -> Option<String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{MAPVK_VK_TO_CHAR, MapVirtualKeyW};
    // SAFETY: a plain lookup with no pointers.
    let code = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_CHAR) } & 0x7FFF;
    let letter = char::from_u32(code).filter(|letter| !letter.is_control() && !letter.is_whitespace())?;
    Some(letter.to_uppercase().collect())
}

#[cfg(not(windows))]
fn layout_name(_vk: u32) -> Option<String> {
    None
}

/// What the hook tells the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The keys of a shortcut went down.
    Pressed(Action),
    /// The keys being recorded were all let go: the shortcut at this place takes them.
    Recorded { index: usize, keys: Vec<u32> },
}

/// What the hook knows, shared between the window and the hook's thread.
#[derive(Default)]
struct State {
    shortcuts: Vec<Shortcut>,
    /// The keys down now, each with the code Windows gave it, to ask Windows about it later.
    down: std::collections::BTreeMap<u32, u32>,
    /// The shortcuts whose keys are held now, by their place.
    active: BTreeSet<usize>,
    recording: Option<Recording>,
    events: Option<Sender<Event>>,
    waker: Option<egui::Context>,
    /// Asks Windows whether the keys thought held still are: a key let go on the lock screen or
    /// over a prompt of the administrator never reaches the hook. Off in the tests.
    check_real: bool,
    /// The last key the hook of the whole system saw, going down or up, and when: the hook of the
    /// window sees the same key right after, and must not count it twice.
    last_global: Option<(u32, bool, std::time::Instant)>,
}

/// Where a key was seen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    /// The hook of the whole system, on its own thread.
    Global,
    /// The hook of the thread of the window, which sees the keys while the window is in front.
    Window,
}

struct Recording {
    index: usize,
    /// The keys pressed since the recording began, in order, and how many are held now.
    keys: Vec<u32>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<R>(work: impl FnOnce(&mut State) -> R) -> Option<R> {
    let mut guard = STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.as_mut().map(work)
}

impl State {
    /// A key went down or up, seen by one of the hooks.
    fn key(&mut self, source: Source, code: u32, raw: u32, down: bool) {
        match source {
            Source::Global => self.last_global = Some((code, down, std::time::Instant::now())),
            Source::Window => {
                if self.last_global.is_some_and(|(seen, was_down, at)| seen == code && was_down == down && at.elapsed() < std::time::Duration::from_secs(1)) {
                    self.last_global = None;
                    return;
                }
            }
        }
        if down {
            self.key_down(code, raw);
        } else {
            self.key_up(code);
        }
    }

    /// A key went down, by the code of the shortcuts and the one Windows gave it.
    fn key_down(&mut self, code: u32, raw: u32) {
        let vk = normalize(code);
        self.forget_stale(vk);
        if self.down.insert(vk, raw).is_some() {
            // The repeat of a held key.
            return;
        }
        if let Some(recording) = &mut self.recording {
            if !recording.keys.contains(&vk) && recording.keys.len() < MAX_KEYS {
                recording.keys.push(vk);
            }
            return;
        }
        self.fire(vk);
    }

    /// Lets go of keys Windows says are up: a key let go on the lock screen or over a prompt of
    /// the administrator never reaches the hook.
    fn forget_stale(&mut self, except: u32) {
        if self.check_real {
            self.down.retain(|key, raw| *key == except || still_down(*raw));
        }
    }

    /// Starts the shortcuts that `vk`, just pressed, completes.
    fn fire(&mut self, vk: u32) {
        let modifiers: Vec<u32> = MODIFIERS.into_iter().filter(|modifier| self.down.contains_key(modifier)).collect();
        for (index, shortcut) in self.shortcuts.iter().enumerate() {
            if !shortcut.enabled || shortcut.keys.is_empty() || self.active.contains(&index) || !shortcut.keys.contains(&vk) {
                continue;
            }
            // The modifiers must be exactly those of the shortcut, so Ctrl+M does not go off
            // with Ctrl+Shift+M; other keys held along do not matter, so push to talk works
            // while walking in a game.
            let wanted: Vec<u32> = shortcut.keys.iter().copied().filter(|key| is_modifier(*key)).collect();
            if wanted != modifiers || !shortcut.keys.iter().all(|key| self.down.contains_key(key)) {
                continue;
            }
            self.active.insert(index);
            if !shortcut.action.held() {
                self.send(Event::Pressed(shortcut.action));
            }
        }
        self.apply_gate();
    }

    fn key_up(&mut self, code: u32) {
        let vk = normalize(code);
        // Some keys, as Print Screen, only ever say that they went up.
        let unseen = self.down.remove(&vk).is_none();
        self.forget_stale(vk);
        if let Some(recording) = &mut self.recording {
            if unseen && !is_modifier(vk) && recording.keys.is_empty() {
                recording.keys.push(vk);
            }
            if self.down.is_empty() && !recording.keys.is_empty() {
                let recording = self.recording.as_ref().expect("recording");
                let (index, keys) = (recording.index, canonical(recording.keys.clone()));
                self.recording = None;
                self.send(Event::Recorded { index, keys });
            }
            return;
        }
        if unseen && !is_modifier(vk) {
            self.down.insert(vk, vk);
            self.fire(vk);
            self.down.remove(&vk);
        }
        let shortcuts = &self.shortcuts;
        self.active.retain(|index| shortcuts.get(*index).is_some_and(|shortcut| !shortcut.keys.contains(&vk)));
        self.apply_gate();
    }

    /// Opens or closes the microphone for push to talk and push to mute.
    fn apply_gate(&self) {
        let live = |action: Action| self.shortcuts.iter().any(|shortcut| shortcut.action == action && shortcut.enabled && !shortcut.keys.is_empty());
        let held = |action: Action| self.active.iter().any(|index| self.shortcuts.get(*index).is_some_and(|shortcut| shortcut.action == action));
        let open = (!live(Action::PushToTalk) || held(Action::PushToTalk)) && !held(Action::PushToMute);
        telinha_voice::audio::set_key_gate(open);
    }

    fn send(&self, event: Event) {
        if let Some(events) = &self.events {
            let _ = events.send(event);
        }
        if let Some(waker) = &self.waker {
            waker.request_repaint();
        }
    }
}

/// The global shortcuts of this app. One per process: dropping it takes the hooks away.
pub struct Hotkeys {
    events: Receiver<Event>,
    thread: u32,
    /// The hooks on the thread of the window, as Windows numbers them.
    window_hooks: Vec<isize>,
}

impl Hotkeys {
    /// Puts the hooks in place: one for the whole system on a thread of its own, and one on the
    /// calling thread, which must be the thread of the window. None when Windows refuses them.
    pub fn start(waker: &egui::Context, shortcuts: &[Shortcut]) -> Option<Self> {
        let (sender, events) = std::sync::mpsc::channel();
        *STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(State { shortcuts: shortcuts.to_vec(), events: Some(sender), waker: Some(waker.clone()), check_real: true, ..State::default() });
        with_state(|state| state.apply_gate());
        let thread = hook_thread()?;
        Some(Self { events, thread, window_hooks: window_hooks() })
    }

    /// The shortcuts changed in the settings.
    pub fn set_shortcuts(&self, shortcuts: &[Shortcut]) {
        with_state(|state| {
            state.shortcuts = shortcuts.to_vec();
            state.active.clear();
            state.apply_gate();
        });
    }

    /// Starts recording the keys of the shortcut at this place, or stops with None.
    pub fn record(&self, index: Option<usize>) {
        with_state(|state| state.recording = index.map(|index| Recording { index, keys: Vec::new() }));
    }

    /// The place being recorded, and the keys pressed so far.
    pub fn recording(&self) -> Option<(usize, Vec<u32>)> {
        with_state(|state| state.recording.as_ref().map(|recording| (recording.index, canonical(recording.keys.clone())))).flatten()
    }

    pub fn next_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        stop_thread(self.thread);
        remove_window_hooks(&self.window_hooks);
        *STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        telinha_voice::audio::set_key_gate(true);
    }
}

#[cfg(windows)]
fn still_down(vk: u32) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    // SAFETY: a plain lookup with no pointers.
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}

#[cfg(not(windows))]
fn still_down(_vk: u32) -> bool {
    true
}

#[cfg(windows)]
fn hook_thread() -> Option<u32> {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL};

    let (ready, answer) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("telinha-atalhos".into())
        .spawn(move || {
            // SAFETY: the hooks are set and taken away on this thread, which pumps the messages
            // that make Windows call them; the procedures are plain functions.
            unsafe {
                let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0);
                let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0);
                if keyboard.is_err() {
                    let _ = ready.send(None);
                    return;
                }
                let _ = ready.send(Some(GetCurrentThreadId()));
                let mut message = MSG::default();
                while GetMessageW(&mut message, None, 0, 0).as_bool() {}
                if let Ok(hook) = keyboard {
                    let _ = UnhookWindowsHookEx(hook);
                }
                if let Ok(hook) = mouse {
                    let _ = UnhookWindowsHookEx(hook);
                }
            }
        })
        .ok()?;
    answer.recv().ok().flatten()
}

#[cfg(not(windows))]
fn hook_thread() -> Option<u32> {
    None
}

/// Hooks the keyboard and the mouse of the calling thread, for while its window is in front.
#[cfg(windows)]
fn window_hooks() -> Vec<isize> {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, WH_KEYBOARD, WH_MOUSE};
    // SAFETY: hooks of this thread with plain functions; they are taken away on drop.
    unsafe {
        let thread = GetCurrentThreadId();
        [SetWindowsHookExW(WH_KEYBOARD, Some(window_keyboard_proc), None, thread), SetWindowsHookExW(WH_MOUSE, Some(window_mouse_proc), None, thread)]
            .into_iter()
            .flatten()
            .map(|hook| hook.0 as isize)
            .collect()
    }
}

#[cfg(not(windows))]
fn window_hooks() -> Vec<isize> {
    Vec::new()
}

#[cfg(windows)]
fn remove_window_hooks(hooks: &[isize]) {
    use windows::Win32::UI::WindowsAndMessaging::{HHOOK, UnhookWindowsHookEx};
    for hook in hooks {
        // SAFETY: hooks this app set, taken away once.
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(*hook as *mut core::ffi::c_void));
        }
    }
}

#[cfg(not(windows))]
fn remove_window_hooks(_hooks: &[isize]) {}

/// The keys the window gets: `wparam` is the key, `lparam` the flags of WM_KEYDOWN.
#[cfg(windows)]
unsafe extern "system" fn window_keyboard_proc(code: i32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{CallNextHookEx, HC_ACTION};
    // HC_ACTION is the message taken from the queue; a message only peeked at comes again.
    if code == HC_ACTION as i32 {
        let raw = wparam.0 as u32;
        let flags = lparam.0 as u32;
        let extended = flags & (1 << 24) != 0;
        let down = flags & (1 << 31) == 0;
        let code = from_hook(raw, extended);
        with_state(|state| state.key(Source::Window, code, raw, down));
    }
    // SAFETY: every key goes on to the window untouched.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// The side buttons and the middle one, clicked on the window.
#[cfg(windows)]
unsafe extern "system" fn window_mouse_proc(code: i32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{CallNextHookEx, HC_ACTION, MOUSEHOOKSTRUCTEX, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_NCXBUTTONDOWN, WM_NCXBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP};
    if code == HC_ACTION as i32 {
        let message = wparam.0 as u32;
        // SAFETY: for HC_ACTION Windows passes a MOUSEHOOKSTRUCTEX in lparam.
        let data = unsafe { (*(lparam.0 as *const MOUSEHOOKSTRUCTEX)).mouseData };
        let button = match message {
            WM_MBUTTONDOWN | WM_MBUTTONUP => Some(0x04),
            WM_XBUTTONDOWN | WM_XBUTTONUP | WM_NCXBUTTONDOWN | WM_NCXBUTTONUP => Some(if data >> 16 == 1 { 0x05 } else { 0x06 }),
            _ => None,
        };
        if let Some(vk) = button {
            let down = matches!(message, WM_MBUTTONDOWN | WM_XBUTTONDOWN | WM_NCXBUTTONDOWN);
            with_state(|state| state.key(Source::Window, vk, vk, down));
        }
    }
    // SAFETY: every click goes on to the window untouched.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(windows)]
fn stop_thread(thread: u32) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
    // SAFETY: a message to a thread of this process; nothing is borrowed.
    unsafe {
        let _ = PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0));
    }
}

#[cfg(not(windows))]
fn stop_thread(_thread: u32) {}

#[cfg(windows)]
unsafe extern "system" fn keyboard_proc(code: i32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{CallNextHookEx, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP};
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION Windows passes a KBDLLHOOKSTRUCT in lparam.
        let data = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let code = from_hook(data.vkCode, data.flags.contains(LLKHF_EXTENDED));
        match wparam.0 as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                with_state(|state| state.key(Source::Global, code, data.vkCode, true));
            }
            WM_KEYUP | WM_SYSKEYUP => {
                with_state(|state| state.key(Source::Global, code, data.vkCode, false));
            }
            _ => {}
        }
    }
    // SAFETY: every key goes on to the rest of Windows untouched.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(windows)]
unsafe extern "system" fn mouse_proc(code: i32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{CallNextHookEx, HC_ACTION, MSLLHOOKSTRUCT, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP};
    if code == HC_ACTION as i32 {
        let message = wparam.0 as u32;
        // SAFETY: for HC_ACTION Windows passes a MSLLHOOKSTRUCT in lparam.
        let data = unsafe { (*(lparam.0 as *const MSLLHOOKSTRUCT)).mouseData };
        // The side buttons and the middle one; the left and right clicks never make a shortcut.
        let button = match message {
            WM_MBUTTONDOWN | WM_MBUTTONUP => Some(0x04),
            WM_XBUTTONDOWN | WM_XBUTTONUP => Some(if data >> 16 == 1 { 0x05 } else { 0x06 }),
            _ => None,
        };
        if let Some(vk) = button {
            let down = matches!(message, WM_MBUTTONDOWN | WM_XBUTTONDOWN);
            with_state(|state| state.key(Source::Global, vk, vk, down));
        }
    }
    // SAFETY: every click goes on to the rest of Windows untouched.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(shortcuts: Vec<Shortcut>) -> (State, Receiver<Event>) {
        let (sender, events) = std::sync::mpsc::channel();
        (State { shortcuts, events: Some(sender), ..State::default() }, events)
    }

    fn keys(action: Action, keys: &[u32]) -> Shortcut {
        Shortcut { action, keys: keys.to_vec(), enabled: true }
    }

    #[test]
    fn a_shortcut_goes_off_once_with_exactly_its_modifiers() {
        let (mut state, events) = state(vec![keys(Action::ToggleMute, &[CTRL, 0x4D])]);
        state.key_down(0xA2, 0xA2);
        state.key_down(0x4D, 0x4D);
        state.key_down(0x4D, 0x4D);
        state.key_up(0x4D);
        state.key_up(0xA2);
        assert_eq!(events.try_iter().collect::<Vec<_>>(), vec![Event::Pressed(Action::ToggleMute)]);
        // With Shift held too it is another shortcut.
        state.key_down(0xA3, 0xA3);
        state.key_down(0xA0, 0xA0);
        state.key_down(0x4D, 0x4D);
        assert!(events.try_recv().is_err());
        state.key_up(0x4D);
        state.key_up(0xA0);
        state.key_up(0xA3);
        // Turned off, nothing happens.
        state.shortcuts[0].enabled = false;
        state.key_down(CTRL, CTRL);
        state.key_down(0x4D, 0x4D);
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn push_to_talk_works_while_other_keys_are_held() {
        let (mut state, _events) = state(vec![keys(Action::PushToTalk, &[0x05])]);
        state.apply_gate();
        assert!(!telinha_voice::audio::key_gate());
        state.key_down(0x57, 0x57);
        state.key_down(0x05, 0x05);
        assert!(telinha_voice::audio::key_gate());
        state.key_up(0x57);
        assert!(telinha_voice::audio::key_gate());
        state.key_up(0x05);
        assert!(!telinha_voice::audio::key_gate());
        state.shortcuts.clear();
        state.apply_gate();
        assert!(telinha_voice::audio::key_gate());
    }

    #[test]
    fn recording_takes_the_keys_once_all_are_let_go() {
        let (mut state, events) = state(vec![Shortcut::new(Action::ToggleDeaf)]);
        state.recording = Some(Recording { index: 0, keys: Vec::new() });
        state.key_down(0x4D, 0x4D);
        state.key_down(0xA1, 0xA1);
        state.key_up(0x4D);
        assert!(events.try_recv().is_err());
        state.key_up(0xA1);
        assert_eq!(events.try_recv(), Ok(Event::Recorded { index: 0, keys: vec![SHIFT, 0x4D] }));
        assert_eq!(describe(&[SHIFT, 0x4D]), "SHIFT + M");
        assert_eq!(describe(&[CTRL, ALT, 0x70]), "CTRL + ALT + F1");
    }

    #[test]
    fn the_numeric_keypad_is_itself_with_num_lock_on_or_off() {
        // Num Lock off: the 2 of the keypad comes as the down arrow, without the extended flag.
        assert_eq!(from_hook(0x28, false), 0x62);
        assert_eq!(from_hook(0x28, true), 0x28);
        assert_eq!(from_hook(0x62, false), 0x62);
        assert_eq!(from_hook(0x0D, true), NUM_ENTER);
        assert_eq!(describe(&[from_hook(0x28, false)]), "NUM 2");
        assert_eq!(describe(&[NUM_ENTER]), "NUM ENTER");
        let (mut state, events) = state(vec![keys(Action::ToggleMute, &[0x62])]);
        state.key_down(from_hook(0x28, false), 0x28);
        assert_eq!(events.try_recv(), Ok(Event::Pressed(Action::ToggleMute)));
        assert_eq!(Shortcut::from_json(&json!({ "action": "toggleMute", "keys": [NUM_ENTER] })).map(|shortcut| shortcut.keys), Some(vec![NUM_ENTER]));
    }

    #[test]
    fn a_key_that_only_says_it_went_up_still_counts() {
        let (mut state, events) = state(vec![keys(Action::ShowWindow, &[0x2C])]);
        state.key_up(0x2C);
        assert_eq!(events.try_recv(), Ok(Event::Pressed(Action::ShowWindow)));
        state.recording = Some(Recording { index: 0, keys: Vec::new() });
        state.key_up(0x2C);
        assert_eq!(events.try_recv(), Ok(Event::Recorded { index: 0, keys: vec![0x2C] }));
    }

    /// Sends F24 through Windows itself and checks the hook hears it. F24 is on no keyboard
    /// and types nothing, so it is safe on a desktop in use.
    #[test]
    #[ignore]
    fn the_hook_hears_keys_sent_to_windows() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY};
        let hotkeys = Hotkeys::start(&egui::Context::default(), &[keys(Action::ShowWindow, &[0x87])]).expect("the hooks are in place");
        let key = |up: bool| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0x87), wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, time: 0, dwExtraInfo: 0 } } };
        let press = || {
            // SAFETY: two plain keyboard inputs of the size Windows expects.
            unsafe { SendInput(&[key(false), key(true)], std::mem::size_of::<INPUT>() as i32) };
            std::thread::sleep(std::time::Duration::from_millis(300));
        };
        press();
        assert_eq!(hotkeys.next_event(), Some(Event::Pressed(Action::ShowWindow)));
        hotkeys.record(Some(0));
        press();
        assert_eq!(hotkeys.next_event(), Some(Event::Recorded { index: 0, keys: vec![0x87] }));
    }

    #[test]
    fn a_key_both_hooks_saw_counts_once() {
        let (mut state, events) = state(vec![keys(Action::ShowWindow, &[0x2C])]);
        state.key(Source::Global, 0x2C, 0x2C, true);
        state.key(Source::Window, 0x2C, 0x2C, true);
        state.key(Source::Global, 0x2C, 0x2C, false);
        state.key(Source::Window, 0x2C, 0x2C, false);
        // Up seen twice would look like a key that only says it went up, and fire again.
        assert_eq!(events.try_iter().collect::<Vec<_>>(), vec![Event::Pressed(Action::ShowWindow)]);
        // With the window in front only its hook sees the key, and that is enough.
        state.key(Source::Window, 0x2C, 0x2C, true);
        state.key(Source::Window, 0x2C, 0x2C, false);
        assert_eq!(events.try_iter().collect::<Vec<_>>(), vec![Event::Pressed(Action::ShowWindow)]);
    }

    #[test]
    fn shortcuts_survive_the_file() {
        let shortcut = Shortcut { action: Action::PushToMute, keys: vec![ALT, 0x43], enabled: false };
        assert_eq!(Shortcut::from_json(&shortcut.to_json()), Some(shortcut));
        assert_eq!(Shortcut::from_json(&json!({ "action": "nope", "keys": [1] })), None);
    }
}
