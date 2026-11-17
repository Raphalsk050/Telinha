//! Answers in a thread and pinned messages reach the others in the conversation. The two hubs
//! talk through a wire in memory instead of the message servers, so the test needs no network.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use telinha_core::chats::Message;
use telinha_core::hub::{Conversation, Hub, HubEvent, Outgoing};
use telinha_core::servers::ChannelKind;
use telinha_core::signaling::Broker;
use tokio::sync::mpsc;

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

/// The next message that arrives or changes, skipping everything else.
async fn next_message(events: &mut mpsc::UnboundedReceiver<HubEvent>, what: &str) -> Message {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv()).await.unwrap_or_else(|_| panic!("nothing came while waiting for {what}")).expect("the hub is open");
        if let HubEvent::ChatMessage { message, .. } | HubEvent::ChatUpdate { message, .. } = event {
            return message;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn answers_in_threads_and_pins_reach_the_others() {
    let (one_folder, two_folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let brokers = || vec![Broker::new("wss://127.0.0.1:1/unused")];
    let (one, mut one_events) = Hub::open(one_folder.path(), brokers());
    let (two, mut two_events) = Hub::open(two_folder.path(), brokers());
    one.set_name("Ana");
    two.set_name("Bia");
    let server = one.create_server("Tópicos");
    let channel = server.channels.iter().find(|channel| channel.kind == ChannelKind::Text).unwrap().id.clone();
    two.join_server(&one.invite_code(&server.id).unwrap()).expect("the invite works");
    wire(&one, &two);
    wire(&two, &one);
    one.signaling().publish_presence(true, None);
    two.signaling().publish_presence(true, None);
    let knows_channel = || two.server(&server.id).is_some_and(|server| server.channels.iter().any(|known| known.id == channel));
    for _ in 0..100 {
        if knows_channel() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(knows_channel(), "who joined learned the channels of the server");
    let conversation = Conversation::channel(&server.id, &channel);

    let root = one.send_chat(&conversation, Outgoing { text: "alguém joga hoje?".into(), ..Outgoing::default() }).unwrap().unwrap();
    assert_eq!(root.thread_id(), None);
    let arrived = next_message(&mut two_events, "the first message").await;
    assert_eq!(arrived.id, root.id);

    // An answer in the thread of the first message carries the thread to the other side.
    let answer = two.send_chat(&conversation, Outgoing { text: "eu jogo".into(), thread: Some(root.id.clone()), ..Outgoing::default() }).unwrap().unwrap();
    assert_eq!(answer.thread_id(), Some(root.id.as_str()));
    let got = loop {
        let message = next_message(&mut one_events, "the answer").await;
        if message.id == answer.id {
            break message;
        }
    };
    assert_eq!((got.thread_id(), got.text.as_str()), (Some(root.id.as_str()), "eu jogo"));
    // A thread that does not exist here is not named.
    let loose = two.send_chat(&conversation, Outgoing { text: "solta".into(), thread: Some("nada-disso".into()), ..Outgoing::default() }).unwrap().unwrap();
    assert_eq!(loose.thread_id(), None);

    // Bia pins the first message, for Ana too, and then unpins it.
    assert!(two.pin_message(&conversation, &root.id, true).is_some());
    let pinned = loop {
        let message = next_message(&mut one_events, "the pin").await;
        if message.id == root.id {
            break message;
        }
    };
    assert!(pinned.is_pinned());
    assert_eq!(pinned.pinned_by(), Some("Bia"));
    assert!(one.history(&conversation).iter().any(|message| message.id == root.id && message.is_pinned()), "the pin is kept");
    assert!(two.pin_message(&conversation, &root.id, false).is_some());
    let unpinned = loop {
        let message = next_message(&mut one_events, "the unpin").await;
        if message.id == root.id {
            break message;
        }
    };
    assert!(!unpinned.is_pinned() && unpinned.pinned_by().is_none());
    // A message that is not there cannot be pinned.
    assert!(one.pin_message(&conversation, "nada-disso", true).is_none());
}
