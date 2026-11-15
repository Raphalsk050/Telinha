//! The real engine swaps a program for a downloaded one, started the way the updater starts it.
//! The "programs" are two small files in a scratch folder, and nothing is opened afterwards.
//!
//!   cargo test -p telinha-core --test updater_swap -- --ignored --nocapture
//!
//! Needs the engine built (`build.ps1 -ExeOnly`).

use std::time::{Duration, Instant};

use telinha_core::engine::resolve_engine;
use telinha_core::updater::{ApplyRequest, apply_staged};

#[test]
#[ignore]
fn the_engine_copies_the_download_over_the_program_once_the_app_is_gone() {
    let helper = resolve_engine().expect("the engine is built");
    let folder = tempfile::tempdir().unwrap();
    let stage = folder.path().join("updates");
    std::fs::create_dir_all(&stage).unwrap();
    let source = stage.join("Telinha-9.9.9.exe");
    let target = folder.path().join("Telinha.exe");
    std::fs::write(&source, b"the new program").unwrap();
    std::fs::write(&target, b"the old program").unwrap();

    // Stands for the app: the swap must wait for it to end.
    let mut app = std::process::Command::new("cmd").args(["/c", "ping -n 3 127.0.0.1 >nul"]).spawn().unwrap();
    let request = ApplyRequest {
        helper,
        source: source.clone(),
        target: target.clone(),
        version: "9.9.9".into(),
        wait_pid: app.id(),
        applied_file: stage.join("applied.txt"),
        failed_file: stage.join("failed.txt"),
        relaunch: false,
        elevate: false,
    };
    apply_staged(&request).unwrap();
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(std::fs::read(&target).unwrap(), b"the old program", "nothing is swapped while the app runs");
    app.wait().unwrap();

    let started = Instant::now();
    while std::fs::read(&target).unwrap() != b"the new program" {
        assert!(started.elapsed() < Duration::from_secs(20), "the program was not swapped; failed.txt says {:?}", std::fs::read_to_string(&request.failed_file));
        std::thread::sleep(Duration::from_millis(100));
    }
    let started = Instant::now();
    while std::fs::read_to_string(&request.applied_file).map(|text| text.trim().to_owned()).ok().as_deref() != Some("9.9.9") {
        assert!(started.elapsed() < Duration::from_secs(10), "the swap left no mark");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!request.failed_file.exists());
    println!("swapped {:.1}s after the app ended; the download {} there", started.elapsed().as_secs_f32(), if source.exists() { "is still" } else { "is no longer" });
    // Gives the helper a moment to end, so the scratch folder can be removed.
    std::thread::sleep(Duration::from_millis(500));
}
