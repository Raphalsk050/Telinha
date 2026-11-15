//! Telinha, the window. Everything that is not drawing lives in `telinha-core`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

use telinha::app::TelinhaApp;
use telinha_core::hub::{Hub, default_data_dir};
use telinha_core::signaling::default_brokers;

const VERSION: &str = env!("TELINHA_VERSION");
use telinha::notify::ICON;

#[cfg(embedded_engine)]
static ENGINE: &[u8] = include_bytes!(env!("TELINHA_ENGINE_FILE"));

/// Writes the engine that came inside this program to a folder of its own, once per version of
/// the engine, and returns where it is.
#[cfg(embedded_engine)]
fn unpack_engine() -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    ENGINE.hash(&mut hasher);
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let folder = base.join("Telinha").join("engine").join(format!("{}-{:016x}", ENGINE.len(), hasher.finish()));
    let file = folder.join("telinha.exe");
    let whole = std::fs::metadata(&file).is_ok_and(|metadata| metadata.len() == ENGINE.len() as u64);
    if !whole {
        std::fs::create_dir_all(&folder).ok()?;
        let temporary = folder.join("telinha.exe.part");
        std::fs::write(&temporary, ENGINE).ok()?;
        std::fs::rename(&temporary, &file).ok()?;
        // Engines of older versions are not needed any more.
        if let Some(siblings) = folder.parent().and_then(|parent| std::fs::read_dir(parent).ok()) {
            for sibling in siblings.flatten().filter(|sibling| sibling.path() != folder) {
                let _ = std::fs::remove_dir_all(sibling.path());
            }
        }
    }
    Some(file)
}

#[cfg(not(embedded_engine))]
fn unpack_engine() -> Option<PathBuf> {
    None
}

/// Makes sure only one Telinha runs per data folder. A second one would show up to everyone as
/// the same person twice. When another is already open, its window is brought to the front.
#[cfg(windows)]
fn only_instance(data_dir: &Path) -> bool {
    use std::hash::{Hash, Hasher};
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow};
    use windows::core::{HSTRING, w};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data_dir.to_string_lossy().to_lowercase().hash(&mut hasher);
    let name = HSTRING::from(format!("Local\\Telinha-{:016x}", hasher.finish()));
    // SAFETY: plain Win32 calls with valid, owned strings. The mutex is kept until the process
    // ends, which is what holds the lock.
    unsafe {
        let mutex = CreateMutexW(None, true, &name);
        let taken = GetLastError() == ERROR_ALREADY_EXISTS;
        if mutex.is_ok() && !taken {
            std::mem::forget(mutex);
            return true;
        }
        if let Ok(window) = FindWindowW(None, w!("Telinha")) {
            if IsIconic(window).as_bool() {
                let _ = ShowWindow(window, SW_RESTORE);
            }
            let _ = SetForegroundWindow(window);
        }
    }
    false
}

#[cfg(not(windows))]
fn only_instance(_data_dir: &Path) -> bool {
    true
}

/// Starts looking for new versions, in the program the release pipeline built. Must be called
/// from inside the tokio runtime.
#[cfg(self_update)]
fn start_updater(hub: &std::sync::Arc<Hub>, data_dir: &Path) -> Option<std::sync::Arc<telinha_core::updater::Updater>> {
    use telinha_core::updater::{Endpoints, UpdateEvent, Updater};
    let target = std::env::current_exe().ok()?;
    let helper = telinha_core::engine::resolve_engine()?;
    let log = std::sync::Arc::clone(hub);
    let last = std::sync::Mutex::new(None);
    let updater = Updater::new(VERSION, &target, &data_dir.join("updates"), &helper, Endpoints::github(), move |event| match event {
        UpdateEvent::Changed(state) => {
            let mut last = last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if *last != Some(state.status) {
                *last = Some(state.status);
                log.log().info(&format!("atualizacao: {:?} {}", state.status, state.version.unwrap_or_default()));
            }
        }
        UpdateEvent::Failed(error) => log.log().error(&format!("atualizacao: {error}")),
    });
    let updater = std::sync::Arc::new(updater);
    updater.start();
    Some(updater)
}

#[cfg(not(self_update))]
fn start_updater(_hub: &std::sync::Arc<Hub>, _data_dir: &Path) -> Option<std::sync::Arc<telinha_core::updater::Updater>> {
    None
}

fn window_icon() -> Option<egui::IconData> {
    let picture = image::load_from_memory(ICON).ok()?.to_rgba8();
    let (width, height) = picture.dimensions();
    Some(egui::IconData { rgba: picture.into_raw(), width, height })
}

fn main() -> eframe::Result {
    let data_dir = default_data_dir();
    if !only_instance(&data_dir) {
        return Ok(());
    }
    if std::env::var_os("TELINHA_EXE").is_none() {
        if let Some(engine) = unpack_engine() {
            // SAFETY: nothing else runs yet, so no other thread reads the environment.
            unsafe { std::env::set_var("TELINHA_EXE", engine) };
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .thread_name("telinha")
        .build()
        .expect("the async runtime starts");
    let guard = runtime.enter();
    let (hub, events) = Hub::open(&data_dir, default_brokers());
    let opened = std::env::current_exe().map_or_else(|_| "?".to_owned(), |exe| exe.display().to_string());
    hub.log().info(&format!("Telinha {VERSION} abrindo de {opened}, interface nativa"));
    // Files too large for the message servers go straight from one computer to the other.
    hub.set_file_link(std::sync::Arc::new(telinha_voice::filelink::WebRtcLink::new(telinha_voice::default_ice_servers())));
    hub.start();
    let updater = start_updater(&hub, &data_dir);
    drop(guard);

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Telinha")
        .with_inner_size([1360.0, 860.0])
        .with_min_inner_size([1040.0, 640.0]);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let handle = runtime.handle().clone();
    let closing = std::sync::Arc::clone(&hub);
    let swap = updater.clone();
    let result = eframe::run_native(
        "Telinha",
        options,
        Box::new(move |creation| {
            #[allow(unused_mut)]
            let mut app = TelinhaApp::new(&creation.egui_ctx, hub, events, handle);
            app.enable_notices();
            if let Some(updater) = updater {
                app.set_updater(updater);
            }
            // Development builds can start with a picture open, to look at the viewer.
            #[cfg(debug_assertions)]
            if let Some(file) = std::env::var_os("TELINHA_DEBUG_PICTURE") {
                if let Ok(bytes) = std::fs::read(&file) {
                    app.open_picture("bytes://debug/picture.png", bytes);
                }
            }
            Ok(Box::new(app))
        }),
    );
    // Leaving the call and saying goodbye lets the others see this app go now, not when its
    // presence expires.
    closing.stop_manual();
    closing.leave_voice();
    runtime.block_on(closing.stop());
    closing.log().info("fechando");
    // A version downloaded while the app was open takes its place now. A swap already asked for
    // by the restart button is not asked again.
    if let Some(updater) = swap {
        updater.apply(std::process::id(), false, false);
    }
    result
}
