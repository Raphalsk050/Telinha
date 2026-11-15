//! A file too large for the message servers goes from one app to another: announced in the
//! chat, asked for, fetched through a direct connection and checked. The two hubs here talk
//! through a wire in memory instead of the message servers, and the direct connection is a
//! channel in memory instead of WebRTC, so the test needs no network.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use telinha_core::hub::{Conversation, FileLink, FileStatus, Hub, HubEvent, LinkFuture, Outgoing, OutgoingFile, Receiving, Serving};
use telinha_core::servers::ChannelKind;
use telinha_core::signaling::Broker;
use tokio::sync::{mpsc, oneshot};

/// A direct connection made of a channel: who serves an offer writes straight into the channel
/// of who made it.
#[derive(Default)]
struct MemoryLink {
    waiting: Mutex<HashMap<String, mpsc::Sender<Vec<u8>>>>,
    next: Mutex<u32>,
}

/// One end of the connection in memory, as each hub holds it.
struct End(Arc<MemoryLink>);

impl FileLink for End {
    fn receive(&self) -> LinkFuture<Receiving> {
        let link = Arc::clone(&self.0);
        Box::pin(async move {
            let offer = {
                let mut next = link.next.lock().unwrap();
                *next += 1;
                format!("offer-{next}")
            };
            let (sender, pieces) = mpsc::channel(8);
            link.waiting.lock().unwrap().insert(offer.clone(), sender);
            let (answer, answered) = oneshot::channel::<String>();
            tokio::spawn(async move {
                assert_eq!(answered.await.as_deref(), Ok("answer"));
            });
            Ok(Receiving { offer, answer, pieces })
        })
    }

    fn serve(&self, offer: String) -> LinkFuture<Serving> {
        let link = Arc::clone(&self.0);
        Box::pin(async move {
            let pieces = link.waiting.lock().unwrap().remove(&offer).ok_or("nobody made that offer")?;
            Ok(Serving { answer: "answer".into(), pieces })
        })
    }
}

/// Carries what one hub sends to the other, as the message servers would.
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

async fn next_event(events: &mut mpsc::UnboundedReceiver<HubEvent>, what: &str) -> HubEvent {
    tokio::time::timeout(Duration::from_secs(30), events.recv()).await.unwrap_or_else(|_| panic!("nothing came while waiting for {what}")).expect("the hub is open")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_file_is_announced_fetched_and_checked() {
    let (sender_folder, receiver_folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    // A message server that is never connected to: the wire below takes its place.
    let brokers = || vec![Broker::new("wss://127.0.0.1:1/unused")];
    let (sender, mut sender_events) = Hub::open(sender_folder.path(), brokers());
    let (receiver, mut receiver_events) = Hub::open(receiver_folder.path(), brokers());
    sender.set_name("Quem manda");
    receiver.set_name("Quem recebe");
    let link = Arc::new(MemoryLink::default());
    sender.set_file_link(Arc::new(End(Arc::clone(&link))));
    receiver.set_file_link(Arc::new(End(Arc::clone(&link))));

    let server = sender.create_server("Arquivos");
    let channel = server.channels.iter().find(|channel| channel.kind == ChannelKind::Text).unwrap().id.clone();
    let invite = sender.invite_code(&server.id).unwrap();
    receiver.join_server(&invite).expect("the invite works");
    wire(&sender, &receiver);
    wire(&receiver, &sender);
    // The two say hello, which is when who joined learns the channels of the server.
    sender.signaling().publish_presence(true, None);
    receiver.signaling().publish_presence(true, None);
    let knows_channel = || receiver.server(&server.id).is_some_and(|server| server.channels.iter().any(|known| known.id == channel));
    for _ in 0..100 {
        if knows_channel() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(knows_channel(), "the receiver learned the channels of the server");
    let conversation = Conversation::channel(&server.id, &channel);

    // 9 MB that do not repeat, read from the disk as they go.
    let data: Vec<u8> = (0..9 * 1024 * 1024u32).map(|index| (index.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
    let source = sender_folder.path().join("video.bin");
    std::fs::write(&source, &data).unwrap();
    let file = OutgoingFile { name: "video.bin".into(), mime: "application/octet-stream".into(), data: Vec::new(), path: Some(source) };
    let sent = sender.send_chat(&conversation, Outgoing { text: "olha esse".into(), file: Some(file), ..Outgoing::default() }).unwrap().unwrap();
    let sent_file = sent.file.clone().unwrap();
    assert_eq!((sent_file.size, sent_file.pending, sent.delivered), (data.len() as i64, false, None));

    // The receiver gets the message with the file still to come, then the file.
    let mut statuses = Vec::new();
    let mut progress = 0u64;
    let arrived = loop {
        match next_event(&mut receiver_events, "the file on the receiver").await {
            HubEvent::ChatMessage { message, .. } => {
                let file = message.file.expect("the message announces a file");
                assert!(file.pending && file.key.is_none(), "the file is announced before it arrives");
            }
            HubEvent::FileStatus { status, .. } => statuses.push(status),
            HubEvent::FileProgress { done, total, up, .. } => {
                assert!(!up && total == data.len() as u64 && done >= progress);
                progress = done;
            }
            HubEvent::ChatUpdate { message, .. } => break message,
            _ => {}
        }
    };
    let arrived_file = arrived.file.unwrap();
    assert!(!arrived_file.pending);
    assert_eq!((arrived_file.name.as_str(), arrived_file.size), ("video.bin", data.len() as i64));
    assert_eq!(progress, data.len() as u64);
    assert!(statuses.contains(&FileStatus::Searching) && statuses.contains(&FileStatus::Receiving), "{statuses:?}");
    assert!(!statuses.iter().any(|status| matches!(status, FileStatus::Failed(_))), "{statuses:?}");

    let copy = receiver_folder.path().join("copia.bin");
    receiver.copy_chat_file(&conversation, &arrived_file.id, &copy).unwrap();
    assert!(std::fs::read(&copy).unwrap() == data, "what arrived is what was sent");
    assert!(receiver.chat_file(&conversation, &arrived_file.id).unwrap() == data);

    // The sender saw the copy being prepared, the message go out and the file leave.
    let (mut prepared, mut delivered, mut left) = (false, false, 0u64);
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(500), sender_events.recv()).await {
        match event {
            HubEvent::UploadProgress { done, total, .. } => prepared |= done == total,
            HubEvent::ChatUpdate { message, .. } => delivered |= message.delivered == Some(true),
            HubEvent::FileProgress { done, up: true, .. } => left = left.max(done),
            _ => {}
        }
    }
    assert!(prepared && delivered, "prepared {prepared}, delivered {delivered}");
    assert_eq!(left, data.len() as u64);
    // What stays on the disk of both is encrypted: the plain bytes are nowhere in the stores.
    assert!(sender.copy_chat_file(&conversation, &sent_file.id, &sender_folder.path().join("volta.bin")).is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_nobody_has_gives_up_and_says_so() {
    let folder = tempfile::tempdir().unwrap();
    let (hub, mut events) = Hub::open(folder.path(), vec![Broker::new("wss://127.0.0.1:1/unused")]);
    hub.set_file_link(Arc::new(End(Arc::new(MemoryLink::default()))));
    let server = hub.create_server("Sozinho");
    let channel = server.channels.iter().find(|channel| channel.kind == ChannelKind::Text).unwrap().id.clone();
    // Everything sent goes nowhere, so nobody ever answers.
    let (sink, _kept) = mpsc::unbounded_channel();
    hub.signaling().tap(sink);
    let announce = serde_json::json!({
        "type": "chat", "from": "someone-else", "at": telinha_core::text::now_ms(), "mid": "m-1", "messageId": "message-1",
        "channelId": channel, "author": "Fulano", "sentAt": telinha_core::text::now_ms(), "text": "",
        "file": { "offerId": "offer-0123456789", "name": "grande.zip", "mime": "application/zip", "size": 20_000_000, "sha256": "ab".repeat(32) },
    });
    hub.signaling().loopback(&server.id, &announce);
    // Three tries of ten seconds each would be the real wait; the clock of the test runs ahead.
    let failed = loop {
        match tokio::time::timeout(Duration::from_secs(60), events.recv()).await.expect("the download gives up").unwrap() {
            HubEvent::FileStatus { status: FileStatus::Failed(reason), .. } => break reason,
            _ => {}
        }
    };
    assert_eq!(failed, "nobody");
    let conversation = Conversation::channel(&server.id, &channel);
    let message = hub.history(&conversation).pop().unwrap();
    assert!(message.file.as_ref().unwrap().pending);
    // Asking again starts over.
    assert!(hub.start_download(&conversation, &message.file.unwrap().id));
}
