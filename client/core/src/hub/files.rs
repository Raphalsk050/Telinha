//! Large files go straight from one computer to the other. The message servers only carry the
//! short talk around it: who wants the file asks who has it (`file-want`), who has it answers
//! (`file-have`), and the two exchange the descriptions of a direct connection (`file-signal`).
//! The bytes go through that connection. Same protocol as `desktop/src/fileshare.js`.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io::Read;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::{Semaphore, mpsc, oneshot};

use super::{Conversation, Hub, HubEvent, public_message};
use crate::chats::FileRef;
use crate::text::new_uuid;
use crate::transfers::is_transfer_id;

const MAX_SDP_LENGTH: usize = 20_000;
const WANT_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Who delivers reads from the disk only as the connection takes it, and on a slow one that
/// takes long.
const SERVE_STALL_TIMEOUT: Duration = Duration::from_secs(120);
const OFFER_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY_DELAY: Duration = Duration::from_secs(3);
const MAX_ATTEMPTS: u32 = 3;
const MAX_DOWNLOADS: usize = 3;
const MAX_SERVES: usize = 6;
/// The size of each piece handed to the connection.
pub const PIECE_BYTES: usize = 64 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

pub type LinkFuture<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;

/// The side that receives a file: its description of the connection, where to hand the answer
/// of the other side, and the pieces as they arrive. Dropping it closes the connection.
pub struct Receiving {
    pub offer: String,
    pub answer: oneshot::Sender<String>,
    /// Ends when the connection closes.
    pub pieces: mpsc::Receiver<Vec<u8>>,
}

/// The side that delivers a file: its answer, and where to put the pieces. Sending waits while
/// the connection is busy, and fails once the connection is gone. Dropping it after the last
/// piece lets what is still on its way arrive.
pub struct Serving {
    pub answer: String,
    pub pieces: mpsc::Sender<Vec<u8>>,
}

/// A direct connection between two computers that carries one file. The app gives the hub one
/// made of WebRTC; the tests give it one made of nothing.
pub trait FileLink: Send + Sync {
    fn receive(&self) -> LinkFuture<Receiving>;
    fn serve(&self, offer: String) -> LinkFuture<Serving>;
}

/// Where a download stands, for the card of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    /// In line behind other downloads.
    Waiting,
    /// Asking who has the file.
    Searching,
    Receiving,
    /// It did not come. The reason is one word: nobody, unreachable, stalled, corrupt, disk,
    /// offline, closed.
    Failed(String),
}

type SignalKey = (String, String, String);

pub(super) struct Files {
    link: Option<Arc<dyn FileLink>>,
    /// Who is waiting for an answer to a `file-want`, by the id of the request.
    wants: HashMap<String, (String, oneshot::Sender<String>)>,
    /// Who is waiting for a connection description, by space, request and the other side.
    signals: HashMap<SignalKey, mpsc::UnboundedSender<String>>,
    serves: HashSet<(String, String)>,
    downloading: HashSet<(String, String)>,
    slots: Arc<Semaphore>,
}

impl Files {
    pub(super) fn has_link(&self) -> bool {
        self.link.is_some()
    }
}

impl Default for Files {
    fn default() -> Self {
        Self { link: None, wants: HashMap::new(), signals: HashMap::new(), serves: HashSet::new(), downloading: HashSet::new(), slots: Arc::new(Semaphore::new(MAX_DOWNLOADS)) }
    }
}

/// What a download is after.
#[derive(Debug, Clone)]
struct Target {
    conversation: Conversation,
    key: String,
    message_id: String,
    file_id: String,
    offer_id: String,
    size: u64,
    sha256: String,
}

/// A file kept on this computer that someone asked for.
struct Local {
    conversation: Conversation,
    key: String,
    file_id: String,
    size: u64,
    secret: (String, String),
}

/// Lets a report out at most every so often, so a fast transfer does not flood the window.
struct Throttle(Option<Instant>);

impl Throttle {
    fn ready(&mut self) -> bool {
        let now = Instant::now();
        if self.0.is_some_and(|last| now.duration_since(last) < PROGRESS_INTERVAL) {
            return false;
        }
        self.0 = Some(now);
        true
    }
}

impl Hub {
    /// Gives the hub the kind of direct connection files travel on. Without one, large files
    /// neither leave nor arrive.
    pub fn set_file_link(&self, link: Arc<dyn FileLink>) {
        self.lock().files.link = Some(link);
    }

    fn file_link(&self) -> Option<Arc<dyn FileLink>> {
        self.lock().files.link.clone()
    }

    fn file_status(&self, target: &Target, status: FileStatus) {
        self.emit(HubEvent::FileStatus { conversation: target.conversation.clone(), file_id: target.file_id.clone(), status });
    }

    /* receiving */

    /// Starts fetching a file that was announced and is not here yet. False when there is
    /// nothing to fetch. Must be called from inside a tokio runtime.
    pub fn start_download(self: &Arc<Self>, conversation: &Conversation, file_id: &str) -> bool {
        let mut inner = self.lock();
        let Some(key) = Self::stored_conversation(&inner, conversation) else { return false };
        let found = inner.chats.history(&key).iter().find_map(|message| {
            let file = message.file.as_ref().filter(|file| file.id == file_id && file.pending)?;
            Some((message.id.clone(), file.clone()))
        });
        let Some((message_id, file)) = found else { return false };
        let (Some(offer_id), Some(sha256)) = (file.offer_id.clone(), file.sha256.clone()) else { return false };
        if !inner.files.downloading.insert((key.clone(), file.id.clone())) {
            return true;
        }
        let slots = Arc::clone(&inner.files.slots);
        drop(inner);
        let target = Target { conversation: conversation.clone(), key, message_id, file_id: file.id, offer_id, size: file.size.max(0) as u64, sha256 };
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            hub.file_status(&target, FileStatus::Waiting);
            let Ok(_slot) = slots.acquire().await else { return };
            let outcome = hub.download(&target).await;
            hub.lock().files.downloading.remove(&(target.key.clone(), target.file_id.clone()));
            if let Err(reason) = outcome {
                hub.file_status(&target, FileStatus::Failed(reason));
            }
        });
        true
    }

    async fn download(self: &Arc<Self>, target: &Target) -> Result<(), String> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            self.file_status(target, FileStatus::Searching);
            match self.download_once(target).await {
                Ok(()) => return Ok(()),
                Err(reason) if attempt >= MAX_ATTEMPTS || reason == "offline" || reason == "gone" => return Err(reason),
                Err(_) => tokio::time::sleep(RETRY_DELAY).await,
            }
        }
    }

    /// One try: finds who has the file, connects, takes the pieces and checks the whole.
    async fn download_once(self: &Arc<Self>, target: &Target) -> Result<(), String> {
        let link = self.file_link().ok_or("unreachable")?;
        let space_id = target.conversation.space_id.clone();
        let request_id = new_uuid();
        let (have, holder) = oneshot::channel();
        self.lock().files.wants.insert(request_id.clone(), (space_id.clone(), have));
        let forget_want = Forget { hub: Arc::clone(self), want: Some(request_id.clone()), signal: None };
        if !self.signaling.publish(&space_id, "file-want", json!({ "offerId": target.offer_id, "requestId": request_id })) {
            return Err("offline".into());
        }
        let holder = tokio::time::timeout(WANT_TIMEOUT, holder).await.ok().and_then(Result::ok).ok_or("nobody")?;
        drop(forget_want);

        let signal_key = (space_id.clone(), request_id.clone(), holder.clone());
        let (signals, mut answers) = mpsc::unbounded_channel();
        self.lock().files.signals.insert(signal_key.clone(), signals);
        let _forget_signal = Forget { hub: Arc::clone(self), want: None, signal: Some(signal_key) };

        let Receiving { offer, answer, mut pieces } = tokio::time::timeout(CONNECT_TIMEOUT, link.receive()).await.ok().and_then(Result::ok).ok_or("unreachable")?;
        if offer.len() > MAX_SDP_LENGTH || !self.signaling.publish(&space_id, "file-signal", json!({ "to": holder, "requestId": request_id, "sdp": offer })) {
            return Err("unreachable".into());
        }
        let sdp = tokio::time::timeout(CONNECT_TIMEOUT, answers.recv()).await.ok().flatten().ok_or("unreachable")?;
        answer.send(sdp).map_err(|_| "unreachable")?;

        let first = tokio::time::timeout(CONNECT_TIMEOUT, pieces.recv()).await.map_err(|_| "unreachable")?.ok_or("closed")?;
        let mut writer = self.lock().chats.create_file_writer(&target.key, &target.file_id).map_err(|_| "disk")?;
        self.file_status(target, FileStatus::Receiving);
        let mut throttle = Throttle(None);
        let mut piece = Some(first);
        let received = loop {
            let Some(data) = piece.take() else { break Err("closed") };
            if writer.size() + data.len() as u64 > target.size {
                break Err("corrupt");
            }
            if writer.write(&data).is_err() {
                break Err("disk");
            }
            let done = writer.size();
            if throttle.ready() || done == target.size {
                self.emit(HubEvent::FileProgress { conversation: target.conversation.clone(), file_id: target.file_id.clone(), done, total: target.size, up: false });
            }
            if done == target.size {
                break Ok(());
            }
            piece = match tokio::time::timeout(STALL_TIMEOUT, pieces.recv()).await {
                Ok(piece) => piece,
                Err(_) => break Err("stalled"),
            };
        };
        if let Err(reason) = received {
            writer.abort();
            return Err(reason.into());
        }
        // The whole file is checked against what the message announced before it counts.
        let stored = tokio::task::spawn_blocking({
            let (size, sha256) = (target.size, target.sha256.clone());
            move || writer.finish(Some((size, &sha256)))
        })
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or("corrupt")?;
        drop(pieces);

        let mut inner = self.lock();
        let updated = inner.chats.update(&target.key, &target.message_id, |message| {
            if let Some(file) = message.file.as_mut().filter(|file| file.id == target.file_id) {
                file.pending = false;
                file.key = Some(stored.key.clone());
                file.iv = Some(stored.iv.clone());
            }
        });
        match updated {
            Some(message) => {
                drop(inner);
                self.emit(HubEvent::ChatUpdate { conversation: target.conversation.clone(), message: public_message(&message) });
                Ok(())
            }
            None => {
                // The message was erased while its file was on the way.
                inner.chats.remove_blob(&target.key, &target.file_id);
                Err("gone".into())
            }
        }
    }

    /* delivering */

    /// A file of this space kept here whole, by the name the message announced it with.
    fn find_shared_file(&self, space_id: &str, offer_id: &str) -> Option<Local> {
        let mut inner = self.lock();
        let conversations: Vec<Conversation> = if inner.contacts.get(space_id).is_some() {
            vec![Conversation::direct(space_id)]
        } else {
            inner.servers.get(space_id).map_or_else(Vec::new, |server| server.channels.iter().map(|channel| Conversation::channel(space_id, &channel.id)).collect())
        };
        for conversation in conversations {
            let key = conversation.key();
            let found = inner.chats.history(&key).iter().find_map(|message| {
                let file: &FileRef = message.file.as_ref().filter(|file| file.offer_id.as_deref() == Some(offer_id) && !file.pending)?;
                Some((file.id.clone(), file.size.max(0) as u64, file.key.clone()?, file.iv.clone()?))
            });
            if let Some((file_id, size, file_key, iv)) = found {
                if inner.chats.has_file(&key, &file_id) {
                    return Some(Local { conversation, key, file_id, size, secret: (file_key, iv) });
                }
            }
        }
        None
    }

    fn handle_file_want(self: &Arc<Self>, space_id: &str, message: &Value) {
        let text = |name: &str| message.get(name).and_then(Value::as_str).unwrap_or("").to_owned();
        let (from, offer_id, request_id) = (text("from"), text("offerId"), text("requestId"));
        if from.is_empty() || !is_transfer_id(&offer_id) || !is_transfer_id(&request_id) {
            return;
        }
        let Some(link) = self.file_link() else { return };
        let Some(local) = self.find_shared_file(space_id, &offer_id) else { return };
        let signal_key = (space_id.to_owned(), request_id.clone(), from.clone());
        let (signals, mut offers) = mpsc::unbounded_channel();
        {
            let mut inner = self.lock();
            if inner.files.serves.len() >= MAX_SERVES || !inner.files.serves.insert((from.clone(), request_id.clone())) {
                return;
            }
            inner.files.signals.insert(signal_key.clone(), signals);
        }
        self.signaling.publish(space_id, "file-have", json!({ "to": from, "requestId": request_id }));

        let hub = Arc::clone(self);
        let space_id = space_id.to_owned();
        tokio::spawn(async move {
            let _forget = Forget { hub: Arc::clone(&hub), want: None, signal: Some(signal_key) };
            let serve = async {
                let offer = tokio::time::timeout(OFFER_TIMEOUT, offers.recv()).await.ok().flatten()?;
                let Serving { answer, pieces } = tokio::time::timeout(CONNECT_TIMEOUT, link.serve(offer)).await.ok()?.ok()?;
                if answer.len() > MAX_SDP_LENGTH || !hub.signaling.publish(&space_id, "file-signal", json!({ "to": from, "requestId": request_id, "sdp": answer })) {
                    return None;
                }
                let reader = hub.lock().chats.open_file_reader(&local.key, &local.file_id, &local.secret.0, &local.secret.1).ok()?;
                let pump = Arc::clone(&hub);
                // The disk is read on a thread of its own, at the pace the connection takes.
                tokio::task::spawn_blocking(move || pump.pump_file(reader, &local, &pieces)).await.ok()
            };
            serve.await;
            hub.lock().files.serves.remove(&(from, request_id));
        });
    }

    fn pump_file(&self, mut reader: impl Read, local: &Local, pieces: &mpsc::Sender<Vec<u8>>) {
        let mut throttle = Throttle(None);
        let mut done = 0u64;
        let started = Instant::now();
        let mut last_progress = started;
        loop {
            let mut piece = vec![0u8; PIECE_BYTES];
            let mut filled = 0;
            // A read may bring less than was asked for before the end of the file.
            while filled < PIECE_BYTES {
                match reader.read(&mut piece[filled..]) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => filled += read,
                }
            }
            if filled == 0 {
                break;
            }
            piece.truncate(filled);
            if pieces.blocking_send(piece).is_err() || last_progress.elapsed() > SERVE_STALL_TIMEOUT {
                return;
            }
            last_progress = Instant::now();
            done += filled as u64;
            if throttle.ready() || done == local.size {
                self.emit(HubEvent::FileProgress { conversation: local.conversation.clone(), file_id: local.file_id.clone(), done, total: local.size, up: true });
            }
        }
    }

    /* the talk between the two ends */

    pub(super) fn handle_file_message(self: &Arc<Self>, space_id: &str, kind: &str, message: &Value) {
        let text = |name: &str| message.get(name).and_then(Value::as_str);
        if kind == "file-want" {
            self.handle_file_want(space_id, message);
            return;
        }
        let (Some(from), Some(request_id)) = (text("from"), text("requestId")) else { return };
        if text("to") != Some(self.instance_id()) {
            return;
        }
        let mut inner = self.lock();
        if kind == "file-have" {
            let waiting = inner.files.wants.get(request_id).is_some_and(|(space, _)| space == space_id);
            if waiting {
                if let Some((_, have)) = inner.files.wants.remove(request_id) {
                    let _ = have.send(from.to_owned());
                }
            }
        } else if let Some(sdp) = text("sdp").filter(|sdp| sdp.len() <= MAX_SDP_LENGTH) {
            if let Some(waiting) = inner.files.signals.get(&(space_id.to_owned(), request_id.to_owned(), from.to_owned())) {
                let _ = waiting.send(sdp.to_owned());
            }
        }
    }
}

/// Takes a request out of the books when its task ends, however it ends.
struct Forget {
    hub: Arc<Hub>,
    want: Option<String>,
    signal: Option<SignalKey>,
}

impl Drop for Forget {
    fn drop(&mut self) {
        let mut inner = self.hub.lock();
        if let Some(want) = &self.want {
            inner.files.wants.remove(want);
        }
        if let Some(signal) = &self.signal {
            inner.files.signals.remove(signal);
        }
    }
}
