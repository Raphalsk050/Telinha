//! The conversations kept on this computer: the last messages of each one, the pictures that came
//! inside messages and the files, all protected on disk. Same files as `desktop/src/chats.js`.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aes::Aes256;
use ctr::cipher::{KeyIvInit, StreamCipher};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::avatars::{is_gif, is_jpeg, is_png, is_webp};
use crate::disk::{lenient_bool, lenient_i64, lenient_string, number_of, write_atomic};
use crate::text::{collapse_spaces, decode_base64, encode_base64, new_uuid, now_ms, random_bytes, truncate_utf16};
use crate::vault::Vault;

pub const MAX_MESSAGES: usize = 500;
pub const MAX_TEXT_LENGTH: usize = 2000;
pub const MAX_IMAGE_BYTES: usize = 400 * 1024;
const MAX_IMAGE_SIDE: i64 = 8192;
const MAX_EMBED_URL: usize = 2048;
const MAX_EMBED_TITLE: usize = 200;
const MAX_EMBED_DESCRIPTION: usize = 400;
const MAX_EMBED_SITE: usize = 80;
const FILE_BUFFER_BYTES: usize = 1024 * 1024;

type FileCipher = ctr::Ctr128BE<Aes256>;

/// Ids of pictures and files, which become part of a file name.
pub fn is_blob_id(text: &str) -> bool {
    (8..=64).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

pub fn clean_text(text: &str) -> String {
    truncate_utf16(text.replace("\r\n", "\n").trim(), MAX_TEXT_LENGTH).to_owned()
}

fn safe_id(id: &str) -> String {
    id.chars().filter(|character| character.is_ascii_alphanumeric() || *character == '_' || *character == '-').collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    pub id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub mime: String,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub width: i64,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub height: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRef {
    pub id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub name: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub mime: String,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub size: i64,
    /// Set while the file has not arrived on this computer yet.
    #[serde(default, deserialize_with = "lenient_bool", skip_serializing_if = "is_false")]
    pub pending: bool,
    /// Names a file that goes straight from one computer to the other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The key and counter the stored file is encrypted with. They never leave this module's
    /// callers: the window gets a message without them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iv: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Embed {
    #[serde(default, deserialize_with = "lenient_string")]
    pub url: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub title: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub description: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub site_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub author: String,
    #[serde(default)]
    pub member_id: Option<String>,
    #[serde(default, deserialize_with = "lenient_bool")]
    pub mine: bool,
    #[serde(default, deserialize_with = "lenient_string")]
    pub text: String,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub sent_at: i64,
    /// Only on messages written here: whether a message server took it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed: Option<Embed>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Message {
    /// Whether this is a call for attention, which shakes the window of whoever gets it.
    pub fn is_nudge(&self) -> bool {
        self.extra.get("nudge").and_then(Value::as_bool) == Some(true)
    }

    /// The ids of everything this message keeps on disk besides its own text.
    pub fn blob_ids(&self) -> Vec<&str> {
        let embed_image = self.embed.as_ref().and_then(|embed| embed.image.as_ref());
        [self.image.as_ref().map(|image| image.id.as_str()), self.file.as_ref().map(|file| file.id.as_str()), embed_image.map(|image| image.id.as_str())]
            .into_iter()
            .flatten()
            .filter(|id| !id.is_empty())
            .collect()
    }
}

/// A picture that came inside a message, already checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub mime: String,
    pub width: i64,
    pub height: i64,
    pub data: Vec<u8>,
}

pub fn image_signature_matches(mime: &str, data: &[u8]) -> Option<bool> {
    match mime {
        "image/webp" => Some(is_webp(data)),
        "image/jpeg" => Some(is_jpeg(data)),
        "image/png" => Some(is_png(data)),
        "image/gif" => Some(is_gif(data)),
        _ => None,
    }
}

pub fn check_image(mime: &str, width: i64, height: i64, data: Vec<u8>) -> Option<DecodedImage> {
    if data.is_empty() || data.len() > MAX_IMAGE_BYTES || !image_signature_matches(mime, &data)? {
        return None;
    }
    let side = |value: i64| value > 0 && value <= MAX_IMAGE_SIDE;
    (side(width) && side(height)).then(|| DecodedImage { mime: mime.to_owned(), width, height, data })
}

pub fn decode_image(image: Option<&Value>) -> Option<DecodedImage> {
    let image = image?.as_object()?;
    let mime = image.get("mime")?.as_str()?;
    let data = decode_base64(image.get("data")?.as_str()?);
    check_image(mime, image.get("width").map_or(0, number_of), image.get("height").map_or(0, number_of), data)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedEmbed {
    pub url: String,
    pub title: String,
    pub description: String,
    pub site_name: String,
    pub image: Option<DecodedImage>,
}

pub fn decode_embed(embed: Option<&Value>) -> Option<DecodedEmbed> {
    let embed = embed?.as_object()?;
    let address = embed.get("url")?.as_str()?;
    if address.len() > MAX_EMBED_URL {
        return None;
    }
    let url = url::Url::parse(address).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    let text = |name: &str, max: usize| embed.get(name).and_then(Value::as_str).map_or_else(String::new, |text| collapse_spaces(text, max));
    let decoded = DecodedEmbed {
        url: url.to_string(),
        title: text("title", MAX_EMBED_TITLE),
        description: text("description", MAX_EMBED_DESCRIPTION),
        site_name: text("siteName", MAX_EMBED_SITE),
        image: decode_image(embed.get("image")),
    };
    (!decoded.title.is_empty() || !decoded.description.is_empty() || decoded.image.is_some()).then_some(decoded)
}

/// What a finished stored file needs to be read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    pub size: u64,
    pub sha256: String,
    pub key: String,
    pub iv: String,
}

/// Writes a large file encrypted with a key of its own, piece by piece, so nothing has to fit
/// whole in memory. The key goes in the conversation, which the vault already protects.
pub struct FileWriter {
    file: PathBuf,
    temporary: PathBuf,
    output: BufWriter<File>,
    cipher: FileCipher,
    hash: Sha256,
    key: [u8; 32],
    iv: [u8; 16],
    size: u64,
}

impl FileWriter {
    fn create(file: PathBuf) -> io::Result<Self> {
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut name = file.as_os_str().to_owned();
        name.push(".part");
        let temporary = PathBuf::from(name);
        let key: [u8; 32] = random_bytes();
        let iv: [u8; 16] = random_bytes();
        Ok(Self {
            output: BufWriter::with_capacity(FILE_BUFFER_BYTES, File::create(&temporary)?),
            cipher: FileCipher::new(&key.into(), &iv.into()),
            hash: Sha256::new(),
            file,
            temporary,
            key,
            iv,
            size: 0,
        })
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn write(&mut self, chunk: &[u8]) -> io::Result<()> {
        self.size += chunk.len() as u64;
        self.hash.update(chunk);
        let mut sealed = chunk.to_vec();
        self.cipher.apply_keystream(&mut sealed);
        self.output.write_all(&sealed)
    }

    /// Closes the file. With `expected`, a file that arrived different from what was announced
    /// is thrown away instead.
    pub fn finish(mut self, expected: Option<(u64, &str)>) -> io::Result<StoredFile> {
        self.output.flush()?;
        drop(self.output);
        let sha256 = hex::encode(self.hash.finalize());
        if expected.is_some_and(|(size, hash)| size != self.size || hash != sha256) {
            let _ = fs::remove_file(&self.temporary);
            return Err(io::Error::other("o arquivo chegou diferente do que foi enviado"));
        }
        fs::rename(&self.temporary, &self.file)?;
        Ok(StoredFile { size: self.size, sha256, key: encode_base64(&self.key), iv: encode_base64(&self.iv) })
    }

    pub fn abort(self) {
        drop(self.output);
        let _ = fs::remove_file(&self.temporary);
    }
}

/// Reads a stored file back, decrypting as it goes.
pub struct FileReader {
    input: BufReader<File>,
    cipher: FileCipher,
}

impl Read for FileReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.input.read(buffer)?;
        self.cipher.apply_keystream(&mut buffer[..read]);
        Ok(read)
    }
}

pub struct ChatStore {
    directory: PathBuf,
    vault: Arc<dyn Vault>,
    cache: HashMap<String, Vec<Message>>,
}

impl ChatStore {
    pub fn new(directory: impl Into<PathBuf>, vault: Arc<dyn Vault>) -> Self {
        Self { directory: directory.into(), vault, cache: HashMap::new() }
    }

    fn file_for(&self, conversation: &str) -> PathBuf {
        self.directory.join(format!("{}.json", safe_id(conversation)))
    }

    fn blob_directory(&self) -> PathBuf {
        self.directory.join("images")
    }

    fn blob_file(&self, conversation: &str, blob_id: &str) -> PathBuf {
        self.blob_directory().join(format!("{}_{}.bin", safe_id(conversation), safe_id(blob_id)))
    }

    fn file_directory(&self) -> PathBuf {
        self.directory.join("files")
    }

    fn stored_file(&self, conversation: &str, blob_id: &str) -> PathBuf {
        self.file_directory().join(format!("{}_{}.dat", safe_id(conversation), safe_id(blob_id)))
    }

    pub fn create_file_writer(&self, conversation: &str, blob_id: &str) -> io::Result<FileWriter> {
        FileWriter::create(self.stored_file(conversation, blob_id))
    }

    pub fn has_file(&self, conversation: &str, blob_id: &str) -> bool {
        self.stored_file(conversation, blob_id).exists()
    }

    pub fn open_file_reader(&self, conversation: &str, blob_id: &str, key: &str, iv: &str) -> io::Result<FileReader> {
        let bad = || io::Error::other("a chave do arquivo guardado esta estragada");
        let key: [u8; 32] = decode_base64(key).try_into().map_err(|_| bad())?;
        let iv: [u8; 16] = decode_base64(iv).try_into().map_err(|_| bad())?;
        let file = File::open(self.stored_file(conversation, blob_id))?;
        Ok(FileReader {
            input: BufReader::with_capacity(FILE_BUFFER_BYTES, file),
            cipher: FileCipher::new(&key.into(), &iv.into()),
        })
    }

    fn load(&mut self, conversation: &str) -> &mut Vec<Message> {
        if !self.cache.contains_key(conversation) {
            let messages = fs::read_to_string(self.file_for(conversation))
                .ok()
                .and_then(|stored| self.vault.decrypt(stored.trim()))
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .and_then(|data| {
                    let list = data.get("messages")?.as_array()?;
                    Some(list.iter().filter_map(|entry| serde_json::from_value(entry.clone()).ok()).collect())
                })
                .unwrap_or_default();
            self.cache.insert(conversation.to_owned(), messages);
        }
        self.cache.get_mut(conversation).expect("the conversation was just loaded")
    }

    pub fn history(&mut self, conversation: &str) -> &[Message] {
        self.load(conversation)
    }

    /// A message written here, not yet stored. Nothing when there is no text and none is allowed
    /// to be missing.
    pub fn create_outgoing(author: &str, text: &str, allow_empty: bool) -> Option<Message> {
        let clean = clean_text(text);
        if clean.is_empty() && !allow_empty {
            return None;
        }
        Some(Message {
            id: new_uuid(),
            author: author.to_owned(),
            member_id: None,
            mine: true,
            text: clean,
            sent_at: now_ms(),
            delivered: None,
            image: None,
            file: None,
            embed: None,
            extra: Map::new(),
        })
    }

    pub fn save_blob(&self, conversation: &str, blob_id: &str, data: &[u8]) -> io::Result<()> {
        let stored = self.vault.encrypt(&encode_base64(data));
        write_atomic(&self.blob_file(conversation, blob_id), stored.as_bytes())
    }

    pub fn load_blob(&self, conversation: &str, blob_id: &str) -> Option<Vec<u8>> {
        let stored = fs::read_to_string(self.blob_file(conversation, blob_id)).ok()?;
        Some(decode_base64(&self.vault.decrypt(stored.trim())?))
    }

    pub fn remove_blob(&self, conversation: &str, blob_id: &str) {
        let _ = fs::remove_file(self.blob_file(conversation, blob_id));
        let stored = self.stored_file(conversation, blob_id);
        let _ = fs::remove_file(&stored);
        let mut part = stored.into_os_string();
        part.push(".part");
        let _ = fs::remove_file(part);
    }

    /// Stores a message in its place by time. False when a message with that id is already there.
    pub fn append(&mut self, conversation: &str, message: Message) -> bool {
        let messages = self.load(conversation);
        if messages.iter().any(|existing| existing.id == message.id) {
            return false;
        }
        messages.push(message);
        messages.sort_by_key(|message| message.sent_at);
        let removed: Vec<Message> = if messages.len() > MAX_MESSAGES {
            let excess = messages.len() - MAX_MESSAGES;
            messages.drain(..excess).collect()
        } else {
            Vec::new()
        };
        for old in &removed {
            for blob_id in old.blob_ids() {
                self.remove_blob(conversation, blob_id);
            }
        }
        self.save(conversation);
        true
    }

    pub fn remove(&mut self, conversation: &str, message_id: &str) -> Option<Message> {
        let messages = self.load(conversation);
        let index = messages.iter().position(|message| message.id == message_id)?;
        let removed = messages.remove(index);
        for blob_id in removed.blob_ids() {
            self.remove_blob(conversation, blob_id);
        }
        self.save(conversation);
        Some(removed)
    }

    pub fn find(&mut self, conversation: &str, message_id: &str) -> Option<&Message> {
        self.load(conversation).iter().find(|message| message.id == message_id)
    }

    pub fn update(&mut self, conversation: &str, message_id: &str, change: impl FnOnce(&mut Message)) -> Option<Message> {
        let message = self.load(conversation).iter_mut().find(|message| message.id == message_id)?;
        change(message);
        let updated = message.clone();
        self.save(conversation);
        Some(updated)
    }

    fn save(&mut self, conversation: &str) {
        let payload = json!({ "version": 1, "messages": self.load(conversation) }).to_string();
        let _ = write_atomic(&self.file_for(conversation), self.vault.encrypt(&payload).as_bytes());
    }

    /// Erases a conversation and everything it kept.
    pub fn forget(&mut self, conversation: &str) {
        self.cache.remove(conversation);
        let _ = fs::remove_file(self.file_for(conversation));
        let prefix = format!("{}_", safe_id(conversation));
        for directory in [self.blob_directory(), self.file_directory()] {
            remove_with_prefix(&directory, &prefix);
        }
    }
}

fn remove_with_prefix(directory: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(directory) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(prefix) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::PlainVault;

    fn store() -> (tempfile::TempDir, ChatStore) {
        let folder = tempfile::tempdir().unwrap();
        let store = ChatStore::new(folder.path().join("chats"), Arc::new(PlainVault));
        (folder, store)
    }

    fn message(id: &str, sent_at: i64) -> Message {
        let mut message = ChatStore::create_outgoing("Rafa", "oi", false).unwrap();
        message.id = id.to_owned();
        message.sent_at = sent_at;
        message
    }

    #[test]
    fn messages_stay_in_time_order_and_an_id_is_stored_once() {
        let (folder, mut chats) = store();
        assert!(chats.append("conversa", message("b", 20)));
        assert!(chats.append("conversa", message("a", 10)));
        assert!(!chats.append("conversa", message("a", 30)));
        let ids: Vec<&str> = chats.history("conversa").iter().map(|message| message.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);

        let mut reloaded = ChatStore::new(folder.path().join("chats"), Arc::new(PlainVault));
        assert_eq!(reloaded.history("conversa").len(), 2);
        assert!(reloaded.update("conversa", "a", |message| message.delivered = Some(true)).is_some());
        assert_eq!(reloaded.find("conversa", "a").unwrap().delivered, Some(true));
        assert!(reloaded.remove("conversa", "a").is_some());
        assert!(reloaded.remove("conversa", "a").is_none());
    }

    #[test]
    fn the_oldest_messages_leave_with_their_pictures_past_the_limit() {
        let (_folder, mut chats) = store();
        let mut first = message("first-message", 1);
        first.image = Some(ImageRef { id: "image-of-first".into(), mime: "image/png".into(), width: 1, height: 1 });
        chats.save_blob("conversa", "image-of-first", b"picture").unwrap();
        chats.append("conversa", first);
        assert_eq!(chats.load_blob("conversa", "image-of-first").unwrap(), b"picture");

        for index in 0..MAX_MESSAGES {
            chats.append("conversa", message(&format!("message-{index}"), 10 + index as i64));
        }
        assert_eq!(chats.history("conversa").len(), MAX_MESSAGES);
        assert!(chats.find("conversa", "first-message").is_none());
        assert!(chats.load_blob("conversa", "image-of-first").is_none());
    }

    #[test]
    fn a_stored_message_from_the_javascript_app_keeps_every_field() {
        let (folder, mut chats) = store();
        let stored = json!({ "version": 1, "messages": [{
            "id": "m1", "author": "Keep", "memberId": "b".repeat(32), "mine": false, "text": "olha",
            "sentAt": 1763150000000.0,
            "image": { "id": "img-12345678", "mime": "image/webp", "width": 640, "height": 480 },
            "file": { "id": "file-12345678", "name": "a.zip", "mime": "application/zip", "size": 9,
                      "offerId": "offer-12345678", "sha256": "ab", "key": "a2V5", "iv": "aXY=" },
            "embed": { "url": "https://example.com/", "title": "t", "description": "", "siteName": "s" },
            "somethingNew": [1, 2],
        }]});
        std::fs::create_dir_all(folder.path().join("chats")).unwrap();
        std::fs::write(folder.path().join("chats").join("conversa.json"), stored.to_string()).unwrap();

        let loaded = chats.history("conversa")[0].clone();
        assert_eq!(loaded.sent_at, 1_763_150_000_000);
        assert_eq!(loaded.file.as_ref().unwrap().offer_id.as_deref(), Some("offer-12345678"));
        assert!(!loaded.file.as_ref().unwrap().pending);
        assert_eq!(loaded.blob_ids(), ["img-12345678", "file-12345678"]);

        chats.update("conversa", "m1", |message| message.text = "mudou".into());
        let raw: Value =
            serde_json::from_str(&std::fs::read_to_string(folder.path().join("chats").join("conversa.json")).unwrap())
                .unwrap();
        let saved = &raw["messages"][0];
        assert_eq!(saved["somethingNew"], json!([1, 2]));
        assert_eq!(saved["file"]["offerId"], "offer-12345678");
        assert_eq!(saved["embed"]["siteName"], "s");
        assert!(saved["file"].get("pending").is_none());
    }

    #[test]
    fn a_large_file_is_stored_encrypted_and_reads_back_the_same() {
        let (folder, chats) = store();
        let content: Vec<u8> = (0..300_000u32).map(|value| (value % 251) as u8).collect();
        let mut writer = chats.create_file_writer("conversa", "file-12345678").unwrap();
        for chunk in content.chunks(70_000) {
            writer.write(chunk).unwrap();
        }
        let stored = writer.finish(None).unwrap();
        assert_eq!(stored.size, content.len() as u64);
        assert_eq!(stored.sha256, hex::encode(Sha256::digest(&content)));
        assert!(chats.has_file("conversa", "file-12345678"));

        let on_disk = std::fs::read(folder.path().join("chats").join("files").join("conversa_file-12345678.dat")).unwrap();
        assert_eq!(on_disk.len(), content.len());
        assert_ne!(on_disk, content);

        let mut read = Vec::new();
        chats.open_file_reader("conversa", "file-12345678", &stored.key, &stored.iv).unwrap().read_to_end(&mut read).unwrap();
        assert_eq!(read, content);

        let mut writer = chats.create_file_writer("conversa", "other-12345678").unwrap();
        writer.write(b"abc").unwrap();
        assert!(writer.finish(Some((3, "wrong"))).is_err());
        assert!(!chats.has_file("conversa", "other-12345678"));
    }

    #[test]
    fn a_picture_in_a_message_is_checked_before_it_is_kept() {
        let png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];
        let good = json!({ "mime": "image/png", "width": 10.9, "height": 20, "data": encode_base64(&png) });
        let decoded = decode_image(Some(&good)).unwrap();
        assert_eq!((decoded.width, decoded.height), (10, 20));
        let wrong_kind = json!({ "mime": "image/gif", "width": 10, "height": 20, "data": encode_base64(&png) });
        assert!(decode_image(Some(&wrong_kind)).is_none());
        let no_size = json!({ "mime": "image/png", "width": 0, "height": 20, "data": encode_base64(&png) });
        assert!(decode_image(Some(&no_size)).is_none());
        assert!(decode_image(Some(&json!({ "mime": "image/svg+xml", "width": 1, "height": 1, "data": "PHN2Zz4=" }))).is_none());
    }

    #[test]
    fn a_link_preview_needs_a_web_address_and_something_to_show() {
        let embed = decode_embed(Some(&json!({ "url": "https://example.com/a b", "title": "  Um   titulo ", "siteName": "x" }))).unwrap();
        assert_eq!(embed.url, "https://example.com/a%20b");
        assert_eq!(embed.title, "Um titulo");
        assert!(decode_embed(Some(&json!({ "url": "javascript:alert(1)", "title": "x" }))).is_none());
        assert!(decode_embed(Some(&json!({ "url": "https://example.com/" }))).is_none());
    }

    #[test]
    fn forgetting_a_conversation_takes_its_files_and_leaves_the_others() {
        let (_folder, mut chats) = store();
        chats.append("uma", message("a", 1));
        chats.append("outra", message("b", 1));
        chats.save_blob("uma", "blob-12345678", b"x").unwrap();
        chats.save_blob("outra", "blob-12345678", b"y").unwrap();
        chats.forget("uma");
        assert!(chats.history("uma").is_empty());
        assert!(chats.load_blob("uma", "blob-12345678").is_none());
        assert_eq!(chats.load_blob("outra", "blob-12345678").unwrap(), b"y");
        assert_eq!(clean_text("  a\r\nb  "), "a\nb");
        assert!(is_blob_id("blob-12345678") && !is_blob_id("short") && !is_blob_id("has space 123"));
    }
}
