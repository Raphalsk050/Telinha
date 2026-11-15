//! Two hubs find each other with codes, with the real engine on both sides: one shares the
//! first monitor, the other watches without a window. Nothing goes through a message server.
//!
//!   cargo test -p telinha-core --test manual_codes -- --ignored --nocapture
//!
//! Needs the engine built (`build.ps1 -ExeOnly`) and a monitor.

use std::time::Duration;

use serde_json::Value;
use telinha_core::engine::{ShareAudio, TargetKind};
use telinha_core::hub::{Hub, HubEvent, ManualRole, ShareChoice};
use tokio::sync::mpsc::UnboundedReceiver;

/// Waits for the next event that `pick` takes, for at most a minute.
async fn wait_for<T>(events: &mut UnboundedReceiver<HubEvent>, what: &str, mut pick: impl FnMut(&HubEvent) -> Option<T>) -> T {
    let search = async {
        loop {
            let event = events.recv().await.unwrap_or_else(|| panic!("the hub closed while waiting for {what}"));
            if let HubEvent::Manual(event) = &event {
                if event["event"] == "error" || event["event"] == "code_rejected" {
                    panic!("the engine complained while waiting for {what}: {event}");
                }
            }
            if let HubEvent::ManualEnded { code, .. } = &event {
                panic!("the engine ended with {code:?} while waiting for {what}");
            }
            if let Some(found) = pick(&event) {
                return found;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), search).await.unwrap_or_else(|_| panic!("nothing came for {what}"))
}

fn code_of(event: &HubEvent, kind: &str) -> Option<String> {
    match event {
        HubEvent::Manual(event) if event["event"] == "code" && event["kind"] == kind => event["code"].as_str().map(str::to_owned),
        _ => None,
    }
}

fn wants(event: &HubEvent, kind: &str) -> Option<()> {
    matches!(event, HubEvent::Manual(event) if event["event"] == "need_code" && event["kind"] == kind).then_some(())
}

fn saved(event: &HubEvent) -> Option<(String, String, bool)> {
    match event {
        HubEvent::ContactSaved { contact_id, name, created } => Some((contact_id.clone(), name.clone(), *created)),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn two_people_meet_with_codes_and_end_up_as_contacts() {
    // SAFETY: set before any engine starts, and nothing else in this test reads the environment.
    unsafe { std::env::set_var("TELINHA_HEADLESS_VIEWER", "1") };
    let (sharer_folder, watcher_folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (sharer, mut sharer_events) = Hub::open(sharer_folder.path(), Vec::new());
    let (watcher, mut watcher_events) = Hub::open(watcher_folder.path(), Vec::new());

    let targets = sharer.share_targets().await.expect("the engine lists what it can share");
    let monitor = targets.monitors.first().and_then(|monitor| monitor.get("handle")).and_then(Value::as_u64).expect("a monitor");
    let choice = ShareChoice { kind: TargetKind::Monitor, handle: monitor, name: "Tela".into(), audio: ShareAudio::None, fps: None };
    sharer.start_manual(ManualRole::Share, Some(choice), "Quem assiste").unwrap();
    assert_eq!(sharer.manual_role(), Some(ManualRole::Share));
    assert!(sharer.start_manual(ManualRole::Watch, None, "x").is_err(), "one connection with codes at a time");
    watcher.start_manual(ManualRole::Watch, None, "Quem transmite").unwrap();

    let invite = wait_for(&mut sharer_events, "the invite", |event| code_of(event, "invite")).await;
    assert!(invite.starts_with("TELINHA1."));
    wait_for(&mut watcher_events, "the watcher to ask for the invite", |event| wants(event, "invite")).await;
    assert!(!watcher.submit_manual_code("not a code"));
    assert!(watcher.submit_manual_code(&format!("  {invite}\n")));
    let answer = wait_for(&mut watcher_events, "the answer", |event| code_of(event, "answer")).await;
    wait_for(&mut sharer_events, "the sharer to ask for the answer", |event| wants(event, "answer")).await;
    assert!(sharer.submit_manual_code(&answer));

    let (sharer_contact, sharer_name, sharer_created) = wait_for(&mut sharer_events, "the sharer to save the contact", saved).await;
    let (watcher_contact, watcher_name, watcher_created) = wait_for(&mut watcher_events, "the watcher to save the contact", saved).await;
    assert!(sharer_created && watcher_created);
    assert_eq!((sharer_name.as_str(), watcher_name.as_str()), ("Quem assiste", "Quem transmite"));
    // Both sides derive the same conversation from the same pair of codes.
    assert_eq!(sharer_contact, watcher_contact);
    assert_eq!(sharer.contacts().len(), 1);
    assert_eq!(watcher.contacts().len(), 1);

    // The picture really flows: the watcher reports frames presented.
    let presented = wait_for(&mut watcher_events, "frames on the watcher", |event| match event {
        HubEvent::Manual(event) if event["event"] == "stats" => event["presented"].as_u64().filter(|frames| *frames > 0),
        _ => None,
    })
    .await;
    println!("the watcher presented {presented} frames");

    sharer.stop_manual();
    watcher.stop_manual();
    let ended = |event: &HubEvent| matches!(event, HubEvent::ManualEnded { stopped: true, .. }).then_some(());
    let wait_end = |events: &mut UnboundedReceiver<HubEvent>| {
        let mut events = std::mem::replace(events, tokio::sync::mpsc::unbounded_channel().1);
        async move {
            tokio::time::timeout(Duration::from_secs(20), async {
                while let Some(event) = events.recv().await {
                    if ended(&event).is_some() {
                        return true;
                    }
                }
                false
            })
            .await
            .unwrap_or(false)
        }
    };
    assert!(wait_end(&mut sharer_events).await, "the sharer's engine stops when asked");
    assert!(wait_end(&mut watcher_events).await, "the watcher's engine stops when asked");
    assert_eq!(sharer.manual_role(), None);
}
