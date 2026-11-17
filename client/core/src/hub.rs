//! The middle of the app: it owns the stores and the signaling, reacts to what arrives and carries
//! out what the window asks. This is what `desktop/main.js` did, without the window.

mod files;
mod manual;
mod streams;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use crate::applog::AppLog;
use crate::avatars::{Picture, PictureStore, check_picture, decode_picture};
use crate::chats::{
    ChatStore, DecodedEmbed, DecodedImage, Embed, FileRef, ImageRef, MAX_TEXT_LENGTH, Message, check_image,
    decode_embed, decode_image, is_blob_id,
};
use crate::contacts::{Contact, ContactStore};
use crate::disk::number_of;
use crate::ports::{PortWindow, random_port_window};
use crate::profile::{ProfileStore, Style};
use crate::servers::{Channel, ChannelKind, SelfMember, Server, ServerStore, is_channel_id, is_member_id};
use crate::signaling::{Broker, Identity, Peer, SignalEvent, Signaling, Status, is_peer_id};
use crate::sounds::SoundStore;
use crate::text::{clean_name, encode_base64, new_uuid, now_ms, truncate_utf16};
use crate::transfers::{
    CHUNK_BYTES, Completed, MAX_FILE_BYTES, MAX_RELAY_BYTES, TransferInbox, clean_file_name, clean_mime, sha256_hex,
    split_file,
};
use crate::vault::{OsVault, PlainVault, Vault};

pub use files::{FileLink, FileStatus, LinkFuture, PIECE_BYTES, Receiving, Serving};
pub use manual::ManualRole;
pub use streams::{SELF_VIEWER, 
    EndReason, IncomingPhase, IncomingView, OutgoingPhase, OutgoingView, ShareChoice, StreamsView, audio_command,
    quality_command, target_command,
};

pub const DM_ROOM: &str = "dm";
/// The version of the call negotiation this app speaks. From 2 on, whoever gets the same offer
/// again answers again instead of restarting the connection.
pub const VOICE_VERSION: i64 = 2;
const MAX_RTC_PAYLOAD: usize = 60_000;
const META_REPLY_INTERVAL_MS: i64 = 5_000;
const PICTURE_REQUEST_INTERVAL_MS: i64 = 60_000;
const RELAY_WINDOW: usize = 4;
const RELAY_ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A conversation: a contact, or one channel of a server.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Conversation {
    pub space_id: String,
    pub channel_id: Option<String>,
}

impl Conversation {
    pub fn direct(space_id: &str) -> Self {
        Self { space_id: space_id.to_owned(), channel_id: None }
    }

    pub fn channel(space_id: &str, channel_id: &str) -> Self {
        Self { space_id: space_id.to_owned(), channel_id: Some(channel_id.to_owned()) }
    }

    /// Names the conversation in the chat store.
    pub fn key(&self) -> String {
        conversation_key(&self.space_id, self.channel_id.as_deref())
    }
}

pub fn conversation_key(space_id: &str, channel_id: Option<&str>) -> String {
    match channel_id {
        Some(channel_id) => format!("{space_id}-{channel_id}"),
        None => space_id.to_owned(),
    }
}

/// The call the person is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceState {
    pub space_id: String,
    pub room_id: String,
    pub mic: bool,
    pub camera: bool,
    pub deaf: bool,
}

/// The screen being shared in the call, as the presence announces it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveState {
    pub space_id: String,
    pub room_id: String,
    pub target_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallKind {
    Call,
    Live,
}

/// What the window is told. It reads the details back from the hub.
#[derive(Debug, Clone, PartialEq)]
pub enum HubEvent {
    Contacts,
    Servers,
    Profile,
    Peers { space_id: String },
    Signaling(Status),
    Voice,
    ChatMessage { conversation: Conversation, message: Message },
    ChatUpdate { conversation: Conversation, message: Message },
    ChatRemove { conversation: Conversation, message_id: String },
    /// A picture of someone arrived or went away.
    Picture { member_id: String, banner: bool },
    /// Something for the Windows notification area, shown only when the person is not looking at
    /// that conversation.
    Notify { title: String, body: String, member_id: Option<String>, open: Conversation },
    IncomingCall { contact_id: String, name: String, kind: CallKind },
    /// A call negotiation message for the call the person is in.
    Rtc { space_id: String, from: String, payload: Value },
    /// What is being shared or watched changed.
    Streams,
    /// The share of this app ended. stopped tells an end the person asked for.
    OutgoingEnded { code: Option<i32>, stopped: bool, error: Option<Value> },
    IncomingEnded { sharer_id: String, name: String, reason: EndReason, error: Option<Value> },
    /// The engine of who shares a screen offered it to be watched inside the window. The id is
    /// `SELF_VIEWER` for the screen this app shares, shown back to the person.
    EmbeddedOffer { sharer_id: String, sdp: String },
    /// A sharer sent a new small picture of their screen.
    StreamThumb { sharer_id: String },
    /// An event of one of the engines, for the statistics panel. The source is outgoing or the
    /// id of the sharer being watched.
    EngineEvent { source: String, event: Value },
    /// Where the download of a large file stands.
    FileStatus { conversation: Conversation, file_id: String, status: FileStatus },
    /// How much of a large file went out or came in through a direct connection.
    FileProgress { conversation: Conversation, file_id: String, done: u64, total: u64, up: bool },
    UploadProgress { conversation: Conversation, file_id: String, done: u64, total: u64 },
    /// An event of the engine of the connection with codes, as the engine wrote it.
    Manual(Value),
    /// The engine of the connection with codes ended. stopped tells an end the person asked for.
    ManualEnded { code: Option<i32>, stopped: bool },
    /// A connection with codes opened, and the person on the other side is now a contact.
    ContactSaved { contact_id: String, name: String, created: bool },
}

/// What a picture inside an outgoing message looks like before it is checked.
#[derive(Debug, Clone)]
pub struct OutgoingImage {
    pub mime: String,
    pub width: i64,
    pub height: i64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct OutgoingFile {
    pub name: String,
    pub mime: String,
    /// The bytes, when the file is in memory.
    pub data: Vec<u8>,
    /// Where the file is, when it is read from the disk as it goes. `data` is then empty.
    pub path: Option<PathBuf>,
}

impl OutgoingFile {
    pub fn size(&self) -> u64 {
        match &self.path {
            Some(path) => std::fs::metadata(path).map_or(0, |metadata| metadata.len()),
            None => self.data.len() as u64,
        }
    }

    /// The whole file in memory. Only for the small ones.
    fn read(&self) -> std::io::Result<Vec<u8>> {
        match &self.path {
            Some(path) => std::fs::read(path),
            None => Ok(self.data.clone()),
        }
    }
}

/// Copies a file about to be sent into the encrypted store, telling how far it got.
fn store_outgoing(mut writer: crate::chats::FileWriter, source: &OutgoingFile, mut progress: impl FnMut(u64)) -> std::io::Result<crate::chats::StoredFile> {
    use std::io::Read;
    const COPY_BYTES: usize = 1024 * 1024;
    let copied = (|| -> std::io::Result<()> {
        match &source.path {
            Some(path) => {
                let mut input = std::fs::File::open(path)?;
                let mut buffer = vec![0u8; COPY_BYTES];
                loop {
                    let read = input.read(&mut buffer)?;
                    if read == 0 {
                        return Ok(());
                    }
                    writer.write(&buffer[..read])?;
                    progress(writer.size());
                }
            }
            None => {
                for chunk in source.data.chunks(COPY_BYTES) {
                    writer.write(chunk)?;
                    progress(writer.size());
                }
                Ok(())
            }
        }
    })();
    match copied {
        Ok(()) if writer.size() > 0 && writer.size() <= MAX_FILE_BYTES => writer.finish(None),
        Ok(()) => {
            writer.abort();
            Err(std::io::Error::other("o arquivo mudou enquanto era lido"))
        }
        Err(error) => {
            writer.abort();
            Err(error)
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Outgoing {
    pub text: String,
    /// A call for attention, as the nudge of MSN: the window of whoever gets it shakes.
    pub nudge: bool,
    /// The message whose thread this one answers.
    pub thread: Option<String>,
    pub image: Option<OutgoingImage>,
    pub file: Option<OutgoingFile>,
    pub embed: Option<Value>,
}

struct Book {
    order: Vec<String>,
    keys: HashMap<String, [u8; 32]>,
    name: String,
    presence: HashMap<String, Map<String, Value>>,
}

/// What the signaling reads about the person, kept apart from the stores so it never waits on
/// their lock.
struct HubIdentity(Arc<RwLock<Book>>);

impl Identity for HubIdentity {
    fn space_ids(&self) -> Vec<String> {
        self.0.read().unwrap_or_else(|poisoned| poisoned.into_inner()).order.clone()
    }

    fn key_for(&self, space_id: &str) -> Option<[u8; 32]> {
        self.0.read().unwrap_or_else(|poisoned| poisoned.into_inner()).keys.get(space_id).copied()
    }

    fn profile_name(&self) -> String {
        self.0.read().unwrap_or_else(|poisoned| poisoned.into_inner()).name.clone()
    }

    fn presence_for(&self, space_id: &str) -> Map<String, Value> {
        let book = self.0.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        book.presence.get(space_id).cloned().unwrap_or_default()
    }
}

struct Inner {
    profile: ProfileStore,
    contacts: ContactStore,
    servers: ServerStore,
    chats: ChatStore,
    avatars: PictureStore,
    banners: PictureStore,
    sounds: SoundStore,
    inbox: TransferInbox,
    streams: streams::Streams,
    files: files::Files,
    manual: Option<manual::Manual>,
    voice: Option<VoiceState>,
    live: Option<LiveState>,
    viewing: Option<String>,
    call_ports: Option<PortWindow>,
    calls: HashMap<String, Option<CallKind>>,
    meta_replies: HashMap<String, i64>,
    picture_requests: HashMap<String, i64>,
}

pub struct Hub {
    data_dir: PathBuf,
    log: AppLog,
    inner: Mutex<Inner>,
    book: Arc<RwLock<Book>>,
    signaling: Arc<Signaling>,
    events: mpsc::UnboundedSender<HubEvent>,
    window_focused: AtomicBool,
}

/// The folder where everything is kept: the one the Electron app used, unless a test or a second
/// profile points somewhere else.
pub fn default_data_dir() -> PathBuf {
    if let Some(folder) = std::env::var_os("TELINHA_DATA_DIR").filter(|folder| !folder.is_empty()) {
        return PathBuf::from(folder);
    }
    let base = std::env::var_os("APPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
    base.join("Telinha")
}

fn system_name() -> String {
    std::env::var("USERNAME").ok().filter(|name| !name.is_empty()).unwrap_or_else(|| "Telinha".into())
}

impl Hub {
    /// Opens the stores of `data_dir`. Returns the hub and the events for the window. Nothing
    /// talks to the network until `start`.
    pub fn open(data_dir: &Path, brokers: Vec<Broker>) -> (Arc<Self>, mpsc::UnboundedReceiver<HubEvent>) {
        let log = AppLog::new(data_dir.join("logs").join("telinha.log"));
        let vault: Arc<dyn Vault> = match OsVault::open(data_dir) {
            Ok(vault) => Arc::new(vault),
            Err(error) => {
                log.error(&format!("cofre: {error}. Os dados novos ficam sem protecao."));
                Arc::new(PlainVault)
            }
        };
        let inner = Inner {
            profile: ProfileStore::load(data_dir.join("profile.json"), &system_name()),
            contacts: ContactStore::load(data_dir.join("contacts.json"), Arc::clone(&vault)),
            servers: ServerStore::load(data_dir.join("servers.json"), Arc::clone(&vault)),
            chats: ChatStore::new(data_dir.join("chats"), Arc::clone(&vault)),
            avatars: PictureStore::load(data_dir.join("avatars")),
            banners: PictureStore::load(data_dir.join("banners")),
            sounds: SoundStore::load(data_dir.join("sounds")),
            inbox: TransferInbox::default(),
            streams: streams::Streams::default(),
            files: files::Files::default(),
            manual: None,
            voice: None,
            live: None,
            viewing: None,
            call_ports: None,
            calls: HashMap::new(),
            meta_replies: HashMap::new(),
            picture_requests: HashMap::new(),
        };
        let book = Arc::new(RwLock::new(Book {
            order: Vec::new(),
            keys: HashMap::new(),
            name: String::new(),
            presence: HashMap::new(),
        }));
        let (signal_sender, signal_events) = mpsc::unbounded_channel();
        let signaling = Signaling::new(Arc::new(HubIdentity(Arc::clone(&book))), brokers, signal_sender);
        let (events, receiver) = mpsc::unbounded_channel();
        let hub = Arc::new(Self {
            data_dir: data_dir.to_owned(),
            log,
            inner: Mutex::new(inner),
            book,
            signaling,
            events,
            window_focused: AtomicBool::new(true),
        });
        hub.sync(&mut hub.lock());
        hub.spawn_signal_loop(signal_events);
        (hub, receiver)
    }

    fn spawn_signal_loop(self: &Arc<Self>, mut signal_events: mpsc::UnboundedReceiver<SignalEvent>) {
        let hub = Arc::downgrade(self);
        let run = async move {
            while let Some(event) = signal_events.recv().await {
                let Some(hub) = hub.upgrade() else { return };
                hub.handle_signal(event);
            }
        };
        // Opened outside a runtime, as tests do, nothing arrives and the loop is not needed.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(run);
        }
    }

    /// Connects to the message servers. Must be called from inside a tokio runtime.
    pub fn start(&self) {
        self.signaling.start();
    }

    pub async fn stop(&self) {
        self.signaling.stop().await;
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn log(&self) -> &AppLog {
        &self.log
    }

    pub fn signaling(&self) -> &Arc<Signaling> {
        &self.signaling
    }

    pub fn instance_id(&self) -> &str {
        self.signaling.instance_id()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn emit(&self, event: HubEvent) {
        let _ = self.events.send(event);
    }

    /// Copies what the signaling needs out of the stores. Called after anything that changes the
    /// spaces, the profile, the call or a server, and always before announcing presence.
    fn sync(&self, inner: &mut Inner) {
        let mut order = Vec::new();
        let mut keys = HashMap::new();
        for contact in inner.contacts.list() {
            if let Some(space) = inner.contacts.keys_for(&contact.id) {
                keys.insert(contact.id.clone(), space.key);
                order.push(contact.id);
            }
        }
        for server in inner.servers.list() {
            if let Some(space) = inner.servers.keys_for(&server.id) {
                keys.insert(server.id.clone(), space.key);
                order.push(server.id);
            }
        }
        let presence = order.iter().map(|id| (id.clone(), presence_for(inner, id))).collect();
        let mut book = self.book.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        *book = Book { order, keys, name: inner.profile.name().to_owned(), presence };
    }

    /* what the window reads */

    pub fn profile_name(&self) -> String {
        self.lock().profile.name().to_owned()
    }

    pub fn member_id(&self) -> String {
        self.lock().profile.member_id().to_owned()
    }

    pub fn profile_style(&self) -> Style {
        self.lock().profile.style().clone()
    }

    pub fn own_picture(&self, banner: bool) -> Option<Picture> {
        let inner = self.lock();
        if banner { inner.profile.banner() } else { inner.profile.avatar() }
    }

    pub fn recent_avatars(&self) -> Vec<Picture> {
        self.lock().profile.recent_avatars()
    }

    /// Contacts with who is online first, then by name.
    pub fn contacts(&self) -> Vec<(Contact, bool)> {
        let mut list: Vec<(Contact, bool)> = self
            .lock()
            .contacts
            .list()
            .into_iter()
            .map(|contact| {
                let online = self.signaling.is_online(&contact.id);
                (contact, online)
            })
            .collect();
        list.sort_by(|(one, one_online), (other, other_online)| {
            other_online.cmp(one_online).then_with(|| one.name.to_lowercase().cmp(&other.name.to_lowercase()))
        });
        list
    }

    pub fn servers(&self) -> Vec<Server> {
        self.lock().servers.list()
    }

    pub fn server(&self, id: &str) -> Option<Server> {
        self.lock().servers.get(id)
    }

    pub fn peers(&self, space_id: &str) -> Vec<Peer> {
        self.signaling.peers(space_id)
    }

    pub fn signaling_status(&self) -> Status {
        self.signaling.status()
    }

    pub fn voice(&self) -> Option<VoiceState> {
        self.lock().voice.clone()
    }

    /// The picture of someone else, by member.
    pub fn picture(&self, member_id: &str, banner: bool) -> Option<Picture> {
        let inner = self.lock();
        if banner { inner.banners.picture(member_id) } else { inner.avatars.picture(member_id) }
    }

    /// The member behind a contact, once their app has announced it.
    pub fn member_of_contact(&self, contact_id: &str) -> Option<String> {
        self.lock().avatars.member_of_contact(contact_id).map(str::to_owned)
    }

    fn stored_conversation(inner: &Inner, conversation: &Conversation) -> Option<String> {
        if inner.contacts.contains(&conversation.space_id) {
            return Some(conversation.space_id.clone());
        }
        let server = inner.servers.get(&conversation.space_id)?;
        let channel_id = conversation.channel_id.as_deref()?;
        server.channels.iter().any(|channel| channel.id == channel_id).then(|| conversation.key())
    }

    /// The messages of a conversation, without the keys of stored files.
    pub fn history(&self, conversation: &Conversation) -> Vec<Message> {
        let mut inner = self.lock();
        let Some(key) = Self::stored_conversation(&inner, conversation) else {
            return Vec::new();
        };
        inner.chats.history(&key).iter().map(public_message).collect()
    }

    /// The bytes of a picture that came in a message or in a link preview.
    pub fn chat_image(&self, conversation: &Conversation, image_id: &str) -> Option<(String, Vec<u8>)> {
        let mut inner = self.lock();
        let key = Self::stored_conversation(&inner, conversation)?;
        if !is_blob_id(image_id) {
            return None;
        }
        let mime = inner.chats.history(&key).iter().find_map(|message| {
            let direct = message.image.as_ref().filter(|image| image.id == image_id);
            let embedded = message.embed.as_ref().and_then(|embed| embed.image.as_ref()).filter(|image| image.id == image_id);
            direct.or(embedded).map(|image| image.mime.clone())
        })?;
        Some((mime, inner.chats.load_blob(&key, image_id)?))
    }

    /// The bytes of a file that went through the message servers and is kept whole.
    pub fn chat_file(&self, conversation: &Conversation, file_id: &str) -> Option<Vec<u8>> {
        use std::io::Read;
        let mut inner = self.lock();
        let key = Self::stored_conversation(&inner, conversation)?;
        if !is_blob_id(file_id) {
            return None;
        }
        let file = inner
            .chats
            .history(&key)
            .iter()
            .find_map(|message| message.file.as_ref().filter(|file| file.id == file_id).cloned())?;
        if file.pending {
            return None;
        }
        match (&file.key, &file.iv) {
            (Some(file_key), Some(iv)) => {
                let mut data = Vec::new();
                inner.chats.open_file_reader(&key, file_id, file_key, iv).ok()?.read_to_end(&mut data).ok()?;
                Some(data)
            }
            _ => inner.chats.load_blob(&key, file_id),
        }
    }

    /* the window tells what the person is looking at */

    pub fn set_window_focused(&self, focused: bool) {
        self.window_focused.store(focused, Ordering::Relaxed);
    }

    pub fn set_viewing(&self, conversation: Option<&Conversation>) {
        let mut inner = self.lock();
        inner.viewing = conversation.and_then(|conversation| Self::stored_conversation(&inner, conversation));
    }

    /* profile */

    fn announce_profile(&self, inner: MutexGuard<'_, Inner>) {
        let mut inner = inner;
        self.sync(&mut inner);
        drop(inner);
        self.signaling.publish_presence(true, None);
        self.emit(HubEvent::Profile);
    }

    pub fn set_name(&self, name: &str) {
        let mut inner = self.lock();
        if !inner.profile.rename(name) {
            return;
        }
        let (member_id, name) = (inner.profile.member_id().to_owned(), inner.profile.name().to_owned());
        for server in inner.servers.list() {
            inner.servers.touch_member(&server.id, &member_id, &name);
        }
        self.announce_profile(inner);
        self.emit(HubEvent::Servers);
    }

    /// Sets the avatar or the banner. False when the picture is not one the app accepts.
    pub fn set_picture(&self, banner: bool, mime: &str, data: Vec<u8>) -> bool {
        let Some(picture) = check_picture(mime, data) else {
            return false;
        };
        let mut inner = self.lock();
        if banner {
            inner.profile.set_banner(&picture);
        } else {
            inner.profile.set_avatar(&picture);
        }
        self.announce_profile(inner);
        true
    }

    pub fn remove_picture(&self, banner: bool) {
        let mut inner = self.lock();
        let removed = if banner { inner.profile.remove_banner() } else { inner.profile.remove_avatar() };
        if removed {
            self.announce_profile(inner);
        }
    }

    pub fn use_recent_avatar(&self, hash: &str) {
        let mut inner = self.lock();
        if inner.profile.use_recent(hash) {
            self.announce_profile(inner);
        }
    }

    pub fn set_style(&self, style: &Value) {
        let mut inner = self.lock();
        if inner.profile.set_style(Some(style)) {
            self.announce_profile(inner);
        }
    }

    /* contacts */

    pub fn rename_contact(&self, id: &str, name: &str) -> bool {
        let renamed = self.lock().contacts.rename(id, name);
        self.emit(HubEvent::Contacts);
        renamed
    }

    pub fn remove_contact(&self, id: &str) -> bool {
        if self.voice().is_some_and(|voice| voice.space_id == id) {
            self.leave_voice();
        }
        self.signaling.remove_space(id);
        let mut inner = self.lock();
        let removed = inner.contacts.remove(id);
        inner.chats.forget(id);
        inner.calls.remove(id);
        self.sync(&mut inner);
        drop(inner);
        self.emit(HubEvent::Contacts);
        removed
    }

    /// Saves the person behind a pair of connection codes as a contact.
    pub fn save_contact_from_codes(&self, invite: &str, answer: &str, name: &str) -> (Contact, bool) {
        let mut inner = self.lock();
        let (contact, created) = inner.contacts.save_from_codes(invite, answer, name);
        self.sync(&mut inner);
        drop(inner);
        if created {
            self.signaling.add_space(&contact.id);
        }
        self.emit(HubEvent::Contacts);
        (contact, created)
    }

    /* servers */

    fn server_changed(&self, inner: MutexGuard<'_, Inner>, server_id: &str) {
        let mut inner = inner;
        self.sync(&mut inner);
        drop(inner);
        self.emit(HubEvent::Servers);
        self.signaling.publish_presence(true, Some(server_id));
    }

    pub fn create_server(&self, name: &str) -> Server {
        let mut inner = self.lock();
        let (member_id, my_name) = (inner.profile.member_id().to_owned(), inner.profile.name().to_owned());
        let server = inner.servers.create(name, Some(SelfMember { member_id: &member_id, name: &my_name }));
        self.sync(&mut inner);
        drop(inner);
        self.signaling.add_space(&server.id);
        self.emit(HubEvent::Servers);
        server
    }

    /// Joins the server of an invite. Nothing when the text holds no valid invite.
    pub fn join_server(&self, code: &str) -> Option<(Server, bool)> {
        let mut inner = self.lock();
        let (member_id, my_name) = (inner.profile.member_id().to_owned(), inner.profile.name().to_owned());
        let (server, created) = inner.servers.join(code, Some(SelfMember { member_id: &member_id, name: &my_name }))?;
        self.sync(&mut inner);
        drop(inner);
        if created {
            self.signaling.add_space(&server.id);
        }
        self.emit(HubEvent::Servers);
        Some((server, created))
    }

    pub fn invite_code(&self, server_id: &str) -> Option<String> {
        self.lock().servers.invite_code(server_id)
    }

    pub fn rename_server(&self, id: &str, name: &str) -> bool {
        let mut inner = self.lock();
        let renamed = inner.servers.rename(id, name);
        if renamed {
            self.server_changed(inner, id);
        }
        renamed
    }

    pub fn set_server_relay(&self, id: &str, relay: Option<&Value>) -> bool {
        let mut inner = self.lock();
        let changed = inner.servers.set_relay(id, relay);
        if changed {
            self.server_changed(inner, id);
        }
        changed
    }

    pub fn leave_server(&self, id: &str) -> bool {
        let Some(server) = self.server(id) else {
            return false;
        };
        if self.voice().is_some_and(|voice| voice.space_id == id) {
            self.leave_voice();
        }
        self.signaling.remove_space(id);
        let mut inner = self.lock();
        for channel in &server.channels {
            inner.chats.forget(&conversation_key(id, Some(&channel.id)));
        }
        inner.servers.leave(id);
        self.sync(&mut inner);
        drop(inner);
        self.emit(HubEvent::Servers);
        true
    }

    pub fn add_channel(&self, server_id: &str, kind: ChannelKind, name: &str) -> Option<Channel> {
        let mut inner = self.lock();
        let channel = inner.servers.add_channel(server_id, kind, name)?;
        self.server_changed(inner, server_id);
        Some(channel)
    }

    pub fn rename_channel(&self, server_id: &str, channel_id: &str, name: &str) -> bool {
        let mut inner = self.lock();
        let renamed = inner.servers.rename_channel(server_id, channel_id, name);
        if renamed {
            self.server_changed(inner, server_id);
        }
        renamed
    }

    pub fn remove_channel(&self, server_id: &str, channel_id: &str) -> bool {
        let mut inner = self.lock();
        if !inner.servers.remove_channel(server_id, channel_id) {
            return false;
        }
        inner.chats.forget(&conversation_key(server_id, Some(channel_id)));
        self.server_changed(inner, server_id);
        self.leave_removed_room(server_id);
        true
    }

    /* call state */

    fn room_exists(inner: &Inner, space_id: &str, room_id: &str) -> bool {
        if inner.contacts.contains(space_id) {
            return room_id == DM_ROOM;
        }
        inner.servers.get(space_id).is_some_and(|server| {
            server.channels.iter().any(|channel| channel.id == room_id && channel.kind == ChannelKind::Voice)
        })
    }

    fn set_voice(&self, inner: MutexGuard<'_, Inner>, next: Option<VoiceState>) {
        let mut inner = inner;
        let previous = std::mem::replace(&mut inner.voice, next.clone());
        if next.is_none() {
            inner.live = None;
        }
        self.sync(&mut inner);
        drop(inner);
        if let Some(previous) = previous.filter(|previous| next.as_ref().is_none_or(|next| next.space_id != previous.space_id)) {
            self.signaling.publish_presence(true, Some(&previous.space_id));
        }
        if let Some(next) = &next {
            self.signaling.publish_presence(true, Some(&next.space_id));
        }
        self.emit(HubEvent::Voice);
    }

    /// Enters a call. Nothing when the room no longer exists.
    pub fn join_voice(&self, space_id: &str, room_id: &str, mic: bool, camera: bool, deaf: bool) -> Option<VoiceState> {
        let moving = self.voice().is_some_and(|voice| voice.space_id != space_id || voice.room_id != room_id);
        if moving {
            self.leave_streams();
        }
        let inner = self.lock();
        if !Self::room_exists(&inner, space_id, room_id) {
            return None;
        }
        let voice = VoiceState { space_id: space_id.to_owned(), room_id: room_id.to_owned(), mic, camera, deaf };
        self.set_voice(inner, Some(voice.clone()));
        Some(voice)
    }

    pub fn update_voice(&self, mic: Option<bool>, camera: Option<bool>, deaf: Option<bool>) -> Option<VoiceState> {
        let inner = self.lock();
        let current = inner.voice.clone()?;
        let next = VoiceState {
            mic: mic.unwrap_or(current.mic),
            camera: camera.unwrap_or(current.camera),
            deaf: deaf.unwrap_or(current.deaf),
            ..current
        };
        self.set_voice(inner, Some(next.clone()));
        Some(next)
    }

    pub fn leave_voice(&self) {
        if self.voice().is_none() {
            return;
        }
        self.leave_streams();
        let inner = self.lock();
        self.set_voice(inner, None);
    }

    fn leave_removed_room(&self, space_id: &str) {
        let inner = self.lock();
        let gone = inner
            .voice
            .as_ref()
            .is_some_and(|voice| voice.space_id == space_id && !Self::room_exists(&inner, space_id, &voice.room_id));
        drop(inner);
        if gone {
            self.leave_voice();
        }
    }

    /// Says in the presence that a screen is being shared in the call, or that it stopped.
    pub fn set_live(&self, live: Option<LiveState>) {
        let mut inner = self.lock();
        if inner.live == live {
            return;
        }
        let space_id = live.as_ref().or(inner.live.as_ref()).map(|live| live.space_id.clone());
        inner.live = live;
        self.sync(&mut inner);
        drop(inner);
        self.signaling.publish_presence(true, space_id.as_deref());
    }

    /// The window of UDP ports for the next connections of a call. The ones already up stay on
    /// their ports.
    pub fn move_call_ports(&self) -> PortWindow {
        let mut inner = self.lock();
        let ports = random_port_window(inner.call_ports);
        inner.call_ports = Some(ports);
        drop(inner);
        self.log.info(&format!("chamada: portas {}-{}", ports.min, ports.max));
        ports
    }

    /// Sends a call negotiation message to one person in the call.
    pub fn send_rtc(&self, space_id: &str, to: &str, payload: &Value) -> bool {
        let Some(voice) = self.voice().filter(|voice| voice.space_id == space_id) else {
            return false;
        };
        if !is_peer_id(to) || payload.to_string().len() > MAX_RTC_PAYLOAD {
            return false;
        }
        self.signaling.publish(space_id, "rtc", json!({ "to": to, "roomId": voice.room_id, "payload": payload }))
    }

    /* what arrives */

    fn handle_signal(self: &Arc<Self>, event: SignalEvent) {
        match event {
            SignalEvent::Status(status) => self.emit(HubEvent::Signaling(status)),
            SignalEvent::Online { .. } => self.emit(HubEvent::Contacts),
            SignalEvent::PeerName { space_id, name } => {
                if self.lock().contacts.adopt_remote_name(&space_id, &name) {
                    self.emit(HubEvent::Contacts);
                }
            }
            SignalEvent::Peers { space_id, peers } => self.handle_peers(&space_id, &peers),
            SignalEvent::PeerPresence { space_id, peer_id, message } => {
                self.handle_peer_presence(&space_id, &peer_id, &message);
            }
            SignalEvent::Message { space_id, message } => self.handle_message(&space_id, message),
        }
    }

    fn handle_peers(&self, space_id: &str, peers: &[Peer]) {
        self.emit(HubEvent::Peers { space_id: space_id.to_owned() });
        if let Some(voice) = self.voice().filter(|voice| voice.space_id == space_id) {
            self.reconcile_streams(space_id, &voice.room_id, peers);
        }

        let mut inner = self.lock();
        let Some(contact) = inner.contacts.get(space_id) else {
            return;
        };
        let in_call: Vec<&Peer> =
            peers.iter().filter(|peer| peer.voice.as_ref().is_some_and(|voice| voice.room_id == DM_ROOM)).collect();
        let next = if in_call.is_empty() {
            None
        } else if in_call.iter().any(|peer| peer.voice.as_ref().is_some_and(|voice| voice.live)) {
            Some(CallKind::Live)
        } else {
            Some(CallKind::Call)
        };
        let previous = inner.calls.insert(space_id.to_owned(), next.clone()).flatten();
        let joined = inner.voice.as_ref().is_some_and(|voice| voice.space_id == space_id);
        let member_id = in_call.first().and_then(|peer| peer.member_id.clone());
        drop(inner);

        let Some(kind) = next else { return };
        if joined || Some(&kind) == previous.as_ref() || (previous == Some(CallKind::Live) && kind == CallKind::Call) {
            return;
        }
        let body = if kind == CallKind::Live { "Está transmitindo a tela para você" } else { "Está chamando você" };
        self.emit(HubEvent::IncomingCall { contact_id: space_id.to_owned(), name: contact.name.clone(), kind });
        self.emit(HubEvent::Notify {
            title: contact.name,
            body: body.to_owned(),
            member_id,
            open: Conversation::direct(space_id),
        });
    }

    fn handle_peer_presence(&self, space_id: &str, peer_id: &str, message: &Value) {
        self.handle_picture_presence(space_id, peer_id, message);

        let mut inner = self.lock();
        if !inner.servers.contains(space_id) {
            return;
        }
        let mut changed = false;
        let mut reply = false;
        if let Some(meta) = message.get("meta").filter(|meta| !meta.is_null()) {
            let result = inner.servers.apply_meta(space_id, meta);
            changed = result.changed;
            if result.mine_newer {
                let now = now_ms();
                let last = inner.meta_replies.get(space_id).copied().unwrap_or(0);
                if now - last >= META_REPLY_INTERVAL_MS {
                    inner.meta_replies.insert(space_id.to_owned(), now);
                    reply = true;
                }
            }
        }
        let member_id = message.get("memberId").and_then(Value::as_str).unwrap_or("");
        let name = message.get("name").and_then(Value::as_str).unwrap_or("");
        changed = inner.servers.touch_member(space_id, member_id, name) || changed;
        if changed {
            self.sync(&mut inner);
        }
        drop(inner);

        if reply {
            self.signaling.publish_presence(true, Some(space_id));
        }
        if changed {
            self.leave_removed_room(space_id);
            self.emit(HubEvent::Servers);
        }
    }

    /// The pictures travel apart from the presence: the presence only says the summary of each
    /// one, and whoever does not have it yet asks. An old app does not know the banner request
    /// and simply does not answer.
    fn handle_picture_presence(&self, space_id: &str, peer_id: &str, message: &Value) {
        let Some(member_id) = message.get("memberId").and_then(Value::as_str).filter(|id| is_member_id(id)) else {
            return;
        };
        let mut inner = self.lock();
        if member_id == inner.profile.member_id() {
            return;
        }
        let mut events = Vec::new();
        let mut requests = Vec::new();
        if inner.contacts.contains(space_id) && inner.avatars.link_contact(space_id, member_id) {
            events.push(HubEvent::Picture { member_id: member_id.to_owned(), banner: false });
        }
        for banner in [false, true] {
            let announced = message.get(if banner { "bannerHash" } else { "avatarHash" }).and_then(Value::as_str);
            let hash = announced.map_or("", |hash| truncate_utf16(hash, 64)).to_owned();
            let Inner { avatars, banners, picture_requests, .. } = &mut *inner;
            let images = if banner { banners } else { avatars };
            if hash.is_empty() {
                if images.remove(member_id) {
                    events.push(HubEvent::Picture { member_id: member_id.to_owned(), banner });
                }
                continue;
            }
            if images.hash_for(member_id) == hash {
                continue;
            }
            let request = format!("{}:{member_id}:{hash}", if banner { "banner" } else { "avatar" });
            let now = now_ms();
            if now - picture_requests.get(&request).copied().unwrap_or(0) < PICTURE_REQUEST_INTERVAL_MS {
                continue;
            }
            picture_requests.insert(request, now);
            let mut body = json!({ "to": peer_id, "hash": hash });
            if banner {
                body["kind"] = "banner".into();
            }
            requests.push(body);
        }
        drop(inner);
        for body in requests {
            self.signaling.publish(space_id, "avatar-request", body);
        }
        for event in events {
            self.emit(event);
        }
    }

    fn handle_message(self: &Arc<Self>, space_id: &str, message: Value) {
        let kind = message.get("type").and_then(Value::as_str).unwrap_or("").to_owned();
        let to_me = message.get("to").and_then(Value::as_str) == Some(self.instance_id());
        match kind.as_str() {
            "avatar-request" if to_me => self.answer_picture_request(space_id, &message),
            "avatar" if to_me => self.store_picture(space_id, &message),
            "avatar-request" | "avatar" => {}
            "file-chunk" => {
                let completed = self.lock().inbox.add_chunk(space_id, &message);
                if let Some(completed) = completed {
                    self.store_completed(completed);
                }
            }
            "file-want" | "file-have" | "file-signal" => self.handle_file_message(space_id, &kind, &message),
            "chat" => self.handle_chat(space_id, &message),
            "chat-delete" => self.handle_chat_delete(space_id, &message),
            "chat-pin" => self.handle_chat_pin(space_id, &message),
            "rtc" => {
                let wanted = self.voice().is_some_and(|voice| {
                    voice.space_id == space_id && message.get("roomId").and_then(Value::as_str) == Some(&voice.room_id)
                });
                if let (true, true, Some(from)) = (to_me, wanted, message.get("from").and_then(Value::as_str)) {
                    let payload = message.get("payload").cloned().unwrap_or(Value::Null);
                    self.emit(HubEvent::Rtc { space_id: space_id.to_owned(), from: from.to_owned(), payload });
                }
            }
            other if other.starts_with("stream-") => self.handle_stream_message(space_id, &message),
            _ => {}
        }
    }

    fn answer_picture_request(&self, space_id: &str, message: &Value) {
        let banner = message.get("kind").and_then(Value::as_str) == Some("banner");
        let inner = self.lock();
        let Some((mime, data, hash)) = inner.profile.picture_base64(banner) else {
            return;
        };
        if message.get("hash").and_then(Value::as_str) != Some(hash) {
            return;
        }
        let mut body = json!({
            "to": message.get("from"),
            "memberId": inner.profile.member_id(),
            "hash": hash,
            "mime": mime,
            "data": data,
        });
        if banner {
            body["kind"] = "banner".into();
        }
        drop(inner);
        self.signaling.publish(space_id, "avatar", body);
    }

    fn store_picture(&self, space_id: &str, message: &Value) {
        let text = |name: &str| message.get(name).and_then(Value::as_str).unwrap_or("");
        let member_id = text("memberId");
        let from = text("from");
        // The picture must come from the person it claims to show.
        let impostor = self
            .signaling
            .peers(space_id)
            .iter()
            .any(|peer| peer.id == from && peer.member_id.as_deref().is_some_and(|known| known != member_id));
        let mut inner = self.lock();
        if !is_member_id(member_id) || member_id == inner.profile.member_id() || impostor {
            return;
        }
        let Some(picture) = decode_picture(text("mime"), text("data")).filter(|picture| picture.hash == text("hash")) else {
            return;
        };
        let banner = text("kind") == "banner";
        let images = if banner { &mut inner.banners } else { &mut inner.avatars };
        if images.put(member_id, &picture) {
            drop(inner);
            self.emit(HubEvent::Picture { member_id: member_id.to_owned(), banner });
        }
    }

    /* chat */

    /// Where a chat message belongs: the conversation and its key in the store.
    fn chat_target(inner: &Inner, space_id: &str, message: &Value) -> Option<Conversation> {
        let message_id = message.get("messageId")?.as_str()?;
        if message_id.len() > 64 {
            return None;
        }
        if inner.contacts.contains(space_id) {
            return Some(Conversation::direct(space_id));
        }
        if !inner.servers.contains(space_id) {
            return None;
        }
        let channel_id = message.get("channelId").and_then(Value::as_str).filter(|id| is_channel_id(id))?;
        Some(Conversation::channel(space_id, channel_id))
    }

    fn handle_chat(self: &Arc<Self>, space_id: &str, message: &Value) {
        if Self::chat_target(&self.lock(), space_id, message).is_none() {
            return;
        }
        // A file without an offer comes in pieces through the message servers: the message waits
        // for all of them.
        let relayed = message.get("file").is_some_and(|file| file.is_object() && file.get("offerId").is_none_or(Value::is_null));
        if relayed {
            let (_, completed) = self.lock().inbox.add_message(space_id, message);
            if let Some(completed) = completed {
                self.store_completed(completed);
            }
            return;
        }
        self.store_incoming(space_id, message, None);
    }

    fn store_completed(self: &Arc<Self>, completed: Completed) {
        self.store_incoming(&completed.space_id, &completed.message, Some(completed.data));
    }

    fn store_incoming(self: &Arc<Self>, space_id: &str, message: &Value, file_data: Option<Vec<u8>>) {
        let mut inner = self.lock();
        let Some(conversation) = Self::chat_target(&inner, space_id, message) else {
            return;
        };
        let key = conversation.key();
        let contact = inner.contacts.get(space_id);
        let text = message.get("text").and_then(Value::as_str).map_or_else(String::new, |text| truncate_utf16(text, MAX_TEXT_LENGTH).to_owned());
        let image = decode_image(message.get("image"));
        let embed = if text.is_empty() { None } else { decode_embed(message.get("embed")) };
        let file = incoming_file(message, file_data.as_deref());
        if text.is_empty() && image.is_none() && file.is_none() {
            return;
        }

        let now = now_ms();
        let author = match &contact {
            Some(contact) => contact.name.clone(),
            None => clean_name(message.get("author").and_then(Value::as_str).unwrap_or(""), "Alguém"),
        };
        let image_ref = image.as_ref().map(image_ref);
        let embed_ref = embed.as_ref().map(embed_ref);
        if let (Some(reference), Some(image)) = (&image_ref, &image) {
            let _ = inner.chats.save_blob(&key, &reference.id, &image.data);
        }
        if let (Some(file), Some(data)) = (&file, &file_data) {
            let _ = inner.chats.save_blob(&key, &file.id, data);
        }
        if let (Some(reference), Some(embed)) = (&embed_ref, &embed) {
            if let (Some(image_reference), Some(image)) = (&reference.image, &embed.image) {
                let _ = inner.chats.save_blob(&key, &image_reference.id, &image.data);
            }
        }
        let sent_at = message.get("sentAt").filter(|value| value.as_f64().is_some_and(f64::is_finite)).map(number_of);
        let stored = Message {
            id: message["messageId"].as_str().unwrap_or_default().to_owned(),
            author: author.clone(),
            member_id: message.get("memberId").and_then(Value::as_str).map(|id| truncate_utf16(id, 32).to_owned()),
            mine: false,
            text,
            sent_at: sent_at.map_or(now, |sent_at| sent_at.min(now + 60_000)),
            delivered: None,
            image: image_ref,
            file,
            embed: embed_ref,
            extra: incoming_extra(message),
        };
        if !inner.chats.append(&key, stored.clone()) {
            for blob_id in stored.blob_ids() {
                inner.chats.remove_blob(&key, blob_id);
            }
            return;
        }

        // A notice goes out whenever the conversation is not in front of the person: the window
        // without focus, or another conversation open.
        let unseen = !self.window_focused.load(Ordering::Relaxed) || inner.viewing.as_deref() != Some(&key);
        let place = inner.servers.get(space_id).map(|server| {
            let channel = conversation.channel_id.as_deref().and_then(|id| server.channels.iter().find(|channel| channel.id == id));
            match channel {
                Some(channel) => format!(" em {} #{}", server.name, channel.name),
                None => format!(" em {}", server.name),
            }
        });
        drop(inner);

        self.emit(HubEvent::ChatMessage { conversation: conversation.clone(), message: public_message(&stored) });
        // A large file was only announced: it is fetched now, straight from who has it. Outside
        // a runtime, as in some tests, it waits for the person to ask.
        if let Some(file) = stored.file.as_ref().filter(|file| file.pending) {
            if tokio::runtime::Handle::try_current().is_ok() {
                self.start_download(&conversation, &file.id);
            }
        }
        if unseen {
            let body = if !stored.text.is_empty() {
                truncate_utf16(&stored.text, 200).to_owned()
            } else if let Some(file) = &stored.file {
                format!("Mandou {}", file.name)
            } else {
                "Mandou uma imagem".to_owned()
            };
            self.emit(HubEvent::Notify {
                title: format!("{author}{}", place.unwrap_or_default()),
                body,
                member_id: stored.member_id.clone(),
                open: conversation,
            });
        }
    }

    /// Only who wrote a message erases it. In a server, whoever asks must be the same member
    /// that sent the message.
    fn handle_chat_delete(&self, space_id: &str, message: &Value) {
        let mut inner = self.lock();
        let Some(conversation) = Self::chat_target(&inner, space_id, message) else {
            return;
        };
        let key = conversation.key();
        let message_id = message["messageId"].as_str().unwrap_or_default().to_owned();
        let Some(stored) = inner.chats.find(&key, &message_id).cloned() else {
            return;
        };
        if stored.mine {
            return;
        }
        if inner.servers.contains(space_id) {
            let member_id = message.get("memberId").and_then(Value::as_str).unwrap_or("");
            let from = message.get("from").and_then(Value::as_str).unwrap_or("");
            let sender_member = self.signaling.peers(space_id).into_iter().find(|peer| peer.id == from).and_then(|peer| peer.member_id);
            if stored.member_id.as_deref() != Some(member_id) || sender_member.is_some_and(|known| known != member_id) {
                return;
            }
        }
        if inner.chats.remove(&key, &message_id).is_some() {
            drop(inner);
            self.emit(HubEvent::ChatRemove { conversation, message_id });
        }
    }

    /// Anyone in a conversation pins or unpins its messages, for everyone in it.
    fn handle_chat_pin(&self, space_id: &str, message: &Value) {
        let mut inner = self.lock();
        let Some(conversation) = Self::chat_target(&inner, space_id, message) else {
            return;
        };
        let key = conversation.key();
        let message_id = message["messageId"].as_str().unwrap_or_default().to_owned();
        let pinned = message.get("pinned").and_then(Value::as_bool) == Some(true);
        let by = match inner.contacts.get(space_id) {
            Some(contact) => contact.name.clone(),
            None => clean_name(message.get("by").and_then(Value::as_str).unwrap_or(""), "Alguém"),
        };
        let changed = inner.chats.find(&key, &message_id).is_some_and(|stored| stored.is_pinned() != pinned);
        if !changed {
            return;
        }
        let updated = inner.chats.update(&key, &message_id, |stored| stored.set_pinned(pinned, &by));
        drop(inner);
        if let Some(updated) = updated {
            self.emit(HubEvent::ChatUpdate { conversation, message: public_message(&updated) });
        }
    }

    /// Pins or unpins a message here and for the others in the conversation. Gives whether a
    /// message server took the change.
    pub fn pin_message(&self, conversation: &Conversation, message_id: &str, pinned: bool) -> Option<bool> {
        let mut inner = self.lock();
        let key = Self::stored_conversation(&inner, conversation)?;
        let by = inner.profile.name().to_owned();
        let updated = inner.chats.update(&key, message_id, |stored| stored.set_pinned(pinned, &by))?;
        let mut body = json!({ "messageId": message_id, "pinned": pinned, "by": by, "memberId": inner.profile.member_id() });
        if inner.servers.contains(&conversation.space_id) {
            body["channelId"] = conversation.channel_id.clone().into();
        }
        drop(inner);
        let delivered = self.signaling.publish(&conversation.space_id, "chat-pin", body);
        self.emit(HubEvent::ChatUpdate { conversation: conversation.clone(), message: public_message(&updated) });
        Some(delivered)
    }

    /// Erases a message here and asks the others to erase it. Whoever is offline at the time
    /// keeps it. Gives whether a message server took the request.
    pub fn delete_message(&self, conversation: &Conversation, message_id: &str) -> Option<bool> {
        let mut inner = self.lock();
        let key = Self::stored_conversation(&inner, conversation)?;
        inner.chats.find(&key, message_id).filter(|message| message.mine)?;
        let mut body = json!({ "messageId": message_id, "memberId": inner.profile.member_id() });
        if inner.servers.contains(&conversation.space_id) {
            body["channelId"] = conversation.channel_id.clone().into();
        }
        inner.chats.remove(&key, message_id);
        drop(inner);
        let delivered = self.signaling.publish(&conversation.space_id, "chat-delete", body);
        self.emit(HubEvent::ChatRemove { conversation: conversation.clone(), message_id: message_id.to_owned() });
        Some(delivered)
    }

    /// Sends a message. The error is a sentence ready to show to the person. A message with
    /// nothing in it gives `Ok(None)`.
    pub fn send_chat(self: &Arc<Self>, conversation: &Conversation, outgoing: Outgoing) -> Result<Option<Message>, String> {
        let mut inner = self.lock();
        let is_server = inner.servers.contains(&conversation.space_id);
        if !is_server && !inner.contacts.contains(&conversation.space_id) {
            return Err("conversa não encontrada".into());
        }
        let key = match (is_server, Self::stored_conversation(&inner, conversation)) {
            (true, None) => return Err("esse canal não existe mais".into()),
            (_, Some(key)) => key,
            (false, None) => conversation.space_id.clone(),
        };
        let image = match outgoing.image {
            Some(image) => Some(
                check_image(&image.mime, image.width, image.height, image.data)
                    .ok_or("essa imagem não pode ser enviada, tente outra")?,
            ),
            None => None,
        };
        let file_size = outgoing.file.as_ref().map_or(0, OutgoingFile::size);
        // A file too large for the message servers goes straight to whoever asks for it.
        let direct = file_size > MAX_RELAY_BYTES as u64;
        if outgoing.file.is_some() {
            if file_size == 0 || file_size > MAX_FILE_BYTES {
                return Err("o arquivo precisa ter no máximo 1 GB".into());
            }
            if direct && !inner.files.has_link() {
                return Err("arquivos acima de 8 MB precisam da conexão direta entre os computadores, que não está disponível".into());
            }
        }
        let preview = decode_embed(outgoing.embed.as_ref());
        let author = inner.profile.name().to_owned();
        let Some(mut message) = ChatStore::create_outgoing(&author, &outgoing.text, image.is_some() || outgoing.file.is_some()) else {
            return Ok(None);
        };

        let image_ref = image.as_ref().map(image_ref);
        let embed_ref = if message.text.is_empty() { None } else { preview.as_ref().map(embed_ref) };
        if let (Some(reference), Some(image)) = (&image_ref, &image) {
            let _ = inner.chats.save_blob(&key, &reference.id, &image.data);
        }
        if let (Some(reference), Some(preview)) = (&embed_ref, &preview) {
            if let (Some(image_reference), Some(image)) = (&reference.image, &preview.image) {
                let _ = inner.chats.save_blob(&key, &image_reference.id, &image.data);
            }
        }
        let file_ref = outgoing.file.as_ref().map(|file| FileRef {
            id: new_uuid(),
            name: clean_file_name(&file.name),
            mime: clean_mime(Some(&file.mime)),
            size: file_size as i64,
            pending: false,
            offer_id: None,
            sha256: None,
            key: None,
            iv: None,
        });
        if let (Some(reference), Some(file), false) = (&file_ref, &outgoing.file, direct) {
            let data = file.read().map_err(|_| "não consegui ler esse arquivo")?;
            let _ = inner.chats.save_blob(&key, &reference.id, &data);
        }

        let member_id = inner.profile.member_id().to_owned();
        let mut payload = json!({
            "messageId": message.id,
            "text": message.text,
            "sentAt": message.sent_at,
            "author": author,
            "memberId": member_id,
        });
        if is_server {
            payload["channelId"] = conversation.channel_id.clone().into();
        }
        if outgoing.nudge {
            payload["nudge"] = true.into();
            message.extra.insert("nudge".into(), true.into());
        }
        // An answer in a thread names the message the thread hangs from, which must be here.
        if let Some(thread) = outgoing.thread.filter(|thread| inner.chats.find(&key, thread).is_some()) {
            payload["threadId"] = thread.clone().into();
            message.extra.insert("threadId".into(), thread.into());
        }
        if let Some(image) = &image {
            payload["image"] = json!({
                "mime": image.mime, "width": image.width, "height": image.height, "data": encode_base64(&image.data),
            });
        }
        if let (Some(_), Some(preview)) = (&embed_ref, &preview) {
            let mut embed = json!({
                "url": preview.url, "title": preview.title, "description": preview.description, "siteName": preview.site_name,
            });
            if let Some(image) = &preview.image {
                embed["image"] = json!({
                    "mime": image.mime, "width": image.width, "height": image.height, "data": encode_base64(&image.data),
                });
            }
            payload["embed"] = embed;
        }

        message.member_id = Some(member_id);
        message.image = image_ref;
        message.file = file_ref.clone();
        message.embed = embed_ref;
        // A message with a file is only delivered once every piece of it went out.
        message.delivered = if file_ref.is_some() { None } else { Some(self.signaling.publish(&conversation.space_id, "chat", payload.clone())) };
        if !inner.chats.append(&key, message.clone()) {
            return Ok(None);
        }
        drop(inner);

        if let (Some(reference), Some(file)) = (file_ref, outgoing.file) {
            let hub = Arc::clone(self);
            let conversation = conversation.clone();
            let message_id = message.id.clone();
            tokio::spawn(async move {
                if direct {
                    hub.offer_file(conversation, key, message_id, payload, reference, file).await;
                } else if let Ok(data) = file.read() {
                    hub.relay_file(conversation, key, message_id, payload, reference, data).await;
                }
            });
        }
        Ok(Some(public_message(&message)))
    }

    /// A large file is first copied, encrypted, to where the app keeps files, and only then
    /// announced: from that moment anyone in the conversation may come and fetch it.
    async fn offer_file(self: Arc<Self>, conversation: Conversation, key: String, message_id: String, payload: Value, file: FileRef, source: OutgoingFile) {
        let total = source.size();
        let writer = self.lock().chats.create_file_writer(&key, &file.id);
        let reporter = Arc::clone(&self);
        let (report_to, file_id) = (conversation.clone(), file.id.clone());
        let stored = tokio::task::spawn_blocking(move || {
            let mut last = std::time::Instant::now();
            store_outgoing(writer?, &source, |done| {
                if last.elapsed() >= std::time::Duration::from_millis(200) || done == total {
                    last = std::time::Instant::now();
                    reporter.emit(HubEvent::UploadProgress { conversation: report_to.clone(), file_id: file_id.clone(), done, total });
                }
            })
        })
        .await
        .ok()
        .and_then(Result::ok);

        let mut delivered = false;
        if let Some(stored) = stored {
            let offer_id = new_uuid();
            let mut inner = self.lock();
            let updated = inner.chats.update(&key, &message_id, |message| {
                if let Some(kept) = message.file.as_mut() {
                    kept.size = stored.size as i64;
                    kept.sha256 = Some(stored.sha256.clone());
                    kept.offer_id = Some(offer_id.clone());
                    kept.key = Some(stored.key.clone());
                    kept.iv = Some(stored.iv.clone());
                }
            });
            if updated.is_some() {
                drop(inner);
                let mut announce = payload;
                announce["file"] = json!({ "offerId": offer_id, "name": file.name, "mime": file.mime, "size": stored.size, "sha256": stored.sha256 });
                delivered = self.signaling.publish(&conversation.space_id, "chat", announce);
            } else {
                // Erased while it was being copied: nothing is announced and the copy goes away.
                inner.chats.remove_blob(&key, &file.id);
                return;
            }
        }
        let updated = self.lock().chats.update(&key, &message_id, |message| message.delivered = Some(delivered));
        if let Some(updated) = updated {
            self.emit(HubEvent::ChatUpdate { conversation, message: public_message(&updated) });
        }
    }

    /// Writes a file kept by the app to a place the person chose, piece by piece, so a large
    /// one never has to fit in memory. The error is a sentence ready to show.
    pub fn copy_chat_file(&self, conversation: &Conversation, file_id: &str, destination: &Path) -> Result<(), String> {
        let gone = || "esse arquivo não está mais guardado neste computador".to_owned();
        let mut inner = self.lock();
        let key = Self::stored_conversation(&inner, conversation).filter(|_| is_blob_id(file_id)).ok_or_else(gone)?;
        let file = inner.chats.history(&key).iter().find_map(|message| message.file.as_ref().filter(|file| file.id == file_id && !file.pending).cloned()).ok_or_else(gone)?;
        match (&file.key, &file.iv) {
            (Some(file_key), Some(iv)) => {
                let mut reader = inner.chats.open_file_reader(&key, file_id, file_key, iv).map_err(|_| gone())?;
                drop(inner);
                let mut output = std::fs::File::create(destination).map_err(|error| error.to_string())?;
                std::io::copy(&mut reader, &mut output).map(|_| ()).map_err(|error| error.to_string())
            }
            _ => {
                let data = inner.chats.load_blob(&key, file_id).ok_or_else(gone)?;
                drop(inner);
                std::fs::write(destination, data).map_err(|error| error.to_string())
            }
        }
    }

    /// Small files go in pieces through the message servers, a few at a time, and each
    /// confirmation from a server counts as progress of the upload.
    async fn relay_file(
        self: Arc<Self>,
        conversation: Conversation,
        key: String,
        message_id: String,
        payload: Value,
        file: FileRef,
        data: Vec<u8>,
    ) {
        let transfer_id = new_uuid();
        let total = split_file(&data).len();
        let space_id = conversation.space_id.clone();
        let (acks, mut acked) = mpsc::unbounded_channel::<bool>();
        let mut next = 0;
        let mut confirmed = 0;
        let mut delivered = true;

        while confirmed < total && delivered {
            // Erased in the middle of the upload: the message must not arrive after the request
            // to erase it.
            if self.lock().chats.find(&key, &message_id).is_none() {
                delivered = false;
                break;
            }
            while next < total && next - confirmed < RELAY_WINDOW {
                let chunk = &data[next * CHUNK_BYTES..data.len().min((next + 1) * CHUNK_BYTES)];
                let body = json!({ "transferId": transfer_id, "index": next, "total": total, "data": encode_base64(chunk) });
                let sender = acks.clone();
                if !self.signaling.publish_acked(&space_id, "file-chunk", body, move |ok| {
                    let _ = sender.send(ok);
                }) {
                    delivered = false;
                    break;
                }
                next += 1;
            }
            if !delivered {
                break;
            }
            match tokio::time::timeout(RELAY_ACK_TIMEOUT, acked.recv()).await {
                Ok(Some(true)) => {
                    confirmed += 1;
                    self.emit(HubEvent::UploadProgress {
                        conversation: conversation.clone(),
                        file_id: file.id.clone(),
                        done: (confirmed * CHUNK_BYTES).min(data.len()) as u64,
                        total: data.len() as u64,
                    });
                }
                _ => delivered = false,
            }
        }

        if delivered {
            let mut announce = payload;
            announce["file"] = json!({
                "transferId": transfer_id, "name": file.name, "mime": file.mime, "size": data.len(), "sha256": sha256_hex(&data),
            });
            delivered = self.signaling.publish(&space_id, "chat", announce);
        }
        let updated = self.lock().chats.update(&key, &message_id, |message| message.delivered = Some(delivered));
        if let Some(updated) = updated {
            self.emit(HubEvent::ChatUpdate { conversation, message: public_message(&updated) });
        }
    }

    /* soundboard */

    pub fn sounds(&self) -> Vec<crate::sounds::Sound> {
        self.lock().sounds.list().to_vec()
    }

    pub fn add_sound(&self, name: &str, mime: &str, data: &[u8]) -> Result<crate::sounds::Sound, String> {
        self.lock().sounds.add(name, mime, data)
    }

    pub fn rename_sound(&self, id: &str, name: &str) -> bool {
        self.lock().sounds.rename(id, name)
    }

    pub fn remove_sound(&self, id: &str) -> bool {
        self.lock().sounds.remove(id)
    }

    pub fn sound_data(&self, id: &str) -> Option<Vec<u8>> {
        self.lock().sounds.read(id)
    }
}

/// What this app says about the person in a space, besides the name.
fn presence_for(inner: &Inner, space_id: &str) -> Map<String, Value> {
    let mut payload = Map::new();
    payload.insert("name".into(), inner.profile.name().into());
    payload.insert("memberId".into(), inner.profile.member_id().into());
    payload.insert("avatarHash".into(), inner.profile.avatar_hash().into());
    if inner.profile.style().is_set() {
        payload.insert("style".into(), serde_json::to_value(inner.profile.style()).unwrap_or(Value::Null));
    }
    if !inner.profile.banner_hash().is_empty() {
        payload.insert("bannerHash".into(), inner.profile.banner_hash().into());
    }
    if let Some(voice) = inner.voice.as_ref().filter(|voice| voice.space_id == space_id) {
        let live = inner.live.as_ref().filter(|live| live.space_id == space_id && live.room_id == voice.room_id);
        payload.insert(
            "voice".into(),
            json!({
                "roomId": voice.room_id,
                "mic": voice.mic,
                "camera": voice.camera,
                "deaf": voice.deaf,
                "live": live.is_some(),
                "liveName": live.map_or("", |live| live.target_name.as_str()),
                "v": VOICE_VERSION,
            }),
        );
    }
    if let Some(meta) = inner.servers.meta(space_id) {
        payload.insert("meta".into(), meta);
    }
    payload
}

/// The key that encrypts a stored file stays in the hub: the window never needs it.
/// What an incoming message carries besides its content: whether it calls for attention and the
/// thread it answers.
fn incoming_extra(message: &Value) -> Map<String, Value> {
    let mut extra = Map::new();
    if message.get("nudge").and_then(Value::as_bool) == Some(true) {
        extra.insert("nudge".to_owned(), Value::Bool(true));
    }
    if let Some(thread) = message.get("threadId").and_then(Value::as_str).filter(|id| !id.is_empty() && id.len() <= 64) {
        extra.insert("threadId".to_owned(), Value::String(thread.to_owned()));
    }
    extra
}

fn public_message(message: &Message) -> Message {
    let mut public = message.clone();
    if let Some(file) = &mut public.file {
        file.key = None;
        file.iv = None;
    }
    public
}

fn image_ref(image: &DecodedImage) -> ImageRef {
    ImageRef { id: new_uuid(), mime: image.mime.clone(), width: image.width, height: image.height }
}

fn embed_ref(embed: &DecodedEmbed) -> Embed {
    Embed {
        url: embed.url.clone(),
        title: embed.title.clone(),
        description: embed.description.clone(),
        site_name: embed.site_name.clone(),
        image: embed.image.as_ref().map(image_ref),
    }
}

/// What is known about the file of a message that arrived: whole when it came through the
/// message servers, or an offer to fetch it straight from whoever has it.
fn incoming_file(message: &Value, data: Option<&[u8]>) -> Option<FileRef> {
    let file = message.get("file")?;
    let name = clean_file_name(file.get("name").and_then(Value::as_str).unwrap_or(""));
    let mime = clean_mime(file.get("mime").and_then(Value::as_str));
    if let Some(data) = data {
        return Some(FileRef {
            id: new_uuid(), name, mime, size: data.len() as i64, pending: false, offer_id: None, sha256: None, key: None, iv: None,
        });
    }
    let offer_id = file.get("offerId")?.as_str().filter(|id| crate::transfers::is_transfer_id(id))?;
    let size = number_of(file.get("size")?);
    let sha256 = file.get("sha256")?.as_str().filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()))?;
    if size < 1 || size as u64 > MAX_FILE_BYTES {
        return None;
    }
    Some(FileRef {
        id: new_uuid(),
        name,
        mime,
        size,
        pending: true,
        offer_id: Some(offer_id.to_owned()),
        sha256: Some(sha256.to_owned()),
        key: None,
        iv: None,
    })
}
