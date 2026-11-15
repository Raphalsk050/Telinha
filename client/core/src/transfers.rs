//! Files that travel in pieces through the message servers, and the rules for file names. Up to
//! 8 MB a file goes this way; above that it goes straight from one computer to the other.
//! Same protocol as `desktop/src/transfers.js`.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::disk::number_of;
use crate::text::{decode_base64, now_ms, truncate_utf16};

pub const CHUNK_BYTES: usize = 192 * 1024;
pub const MAX_RELAY_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_CHUNKS: usize = MAX_RELAY_BYTES.div_ceil(CHUNK_BYTES);
const TRANSFER_TTL_MS: i64 = 3 * 60 * 1000;
const MAX_TRANSFERS: usize = 24;

static TRANSFER_ID_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]{8,64}$").unwrap());
static MIME_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\w.+-]+/[\w.+-]+$").unwrap());

pub fn is_transfer_id(text: &str) -> bool {
    TRANSFER_ID_PATTERN.is_match(text)
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn split_file(data: &[u8]) -> Vec<&[u8]> {
    data.chunks(CHUNK_BYTES).collect()
}

/// A file name that is safe to show and to suggest when saving: no folders, no characters
/// Windows refuses, no leading dots.
pub fn clean_file_name(name: &str) -> String {
    let base = name.rsplit(['\\', '/']).next().unwrap_or("");
    let replaced: String = base
        .chars()
        .map(|character| if character < ' ' || "<>:\"|?*".contains(character) { '_' } else { character })
        .collect();
    let trimmed = replaced.trim_start_matches(|character: char| character == '.' || character.is_whitespace()).trim();
    let clean = truncate_utf16(trimmed, 120);
    if clean.is_empty() { "arquivo".to_owned() } else { clean.to_owned() }
}

pub fn clean_mime(mime: Option<&str>) -> String {
    match mime.filter(|mime| MIME_PATTERN.is_match(mime)) {
        Some(mime) => truncate_utf16(mime, 100).to_owned(),
        None => "application/octet-stream".to_owned(),
    }
}

struct Transfer {
    total: usize,
    chunks: Vec<Option<Vec<u8>>>,
    received: usize,
    bytes: usize,
    message: Option<Value>,
    created_at: i64,
}

/// A chat message whose file has fully arrived, with the file.
pub struct Completed {
    pub space_id: String,
    pub message: Value,
    pub data: Vec<u8>,
}

/// Puts back together the files that arrive in pieces. The pieces come first and the chat message
/// that announces the file comes last, but either order works.
#[derive(Default)]
pub struct TransferInbox {
    transfers: HashMap<String, Transfer>,
}

impl TransferInbox {
    fn entry(&mut self, space_id: &str, from: &str, transfer_id: &str, total: usize) -> Option<(&mut Transfer, String)> {
        let key = format!("{space_id}/{from}/{transfer_id}");
        if !self.transfers.contains_key(&key) {
            let now = now_ms();
            self.transfers.retain(|_, transfer| now - transfer.created_at <= TRANSFER_TTL_MS);
            if self.transfers.len() >= MAX_TRANSFERS {
                return None;
            }
            self.transfers.insert(
                key.clone(),
                Transfer { total, chunks: vec![None; total], received: 0, bytes: 0, message: None, created_at: now },
            );
        }
        let transfer = self.transfers.get_mut(&key)?;
        (transfer.total == total).then_some((transfer, key))
    }

    pub fn add_chunk(&mut self, space_id: &str, message: &Value) -> Option<Completed> {
        let transfer_id = message.get("transferId")?.as_str().filter(|id| is_transfer_id(id))?;
        let from = message.get("from")?.as_str()?;
        let total = usize::try_from(message.get("total")?.as_i64()?).ok().filter(|total| (1..=MAX_CHUNKS).contains(total))?;
        let index = usize::try_from(message.get("index")?.as_i64()?).ok().filter(|index| *index < total)?;
        let data = message.get("data")?.as_str()?;

        let (transfer, key) = self.entry(space_id, from, transfer_id, total)?;
        if transfer.chunks[index].is_some() {
            return None;
        }
        let chunk = decode_base64(data);
        if chunk.is_empty() || chunk.len() > CHUNK_BYTES || transfer.bytes + chunk.len() > MAX_RELAY_BYTES {
            self.transfers.remove(&key);
            return None;
        }
        transfer.received += 1;
        transfer.bytes += chunk.len();
        transfer.chunks[index] = Some(chunk);
        self.try_complete(space_id, &key)
    }

    /// Takes the chat message that announces a relayed file. False when it does not look like one.
    pub fn add_message(&mut self, space_id: &str, message: &Value) -> (bool, Option<Completed>) {
        let accepted = (|| {
            let file = message.get("file")?;
            let transfer_id = file.get("transferId")?.as_str().filter(|id| is_transfer_id(id))?;
            let from = message.get("from")?.as_str()?;
            let size = usize::try_from(number_of(file.get("size")?)).ok().filter(|size| (1..=MAX_RELAY_BYTES).contains(size))?;
            file.get("sha256")?.as_str()?;
            let (transfer, key) = self.entry(space_id, from, transfer_id, size.div_ceil(CHUNK_BYTES))?;
            transfer.message = Some(message.clone());
            Some(key)
        })();
        match accepted {
            Some(key) => (true, self.try_complete(space_id, &key)),
            None => (false, None),
        }
    }

    fn try_complete(&mut self, space_id: &str, key: &str) -> Option<Completed> {
        let transfer = self.transfers.get(key)?;
        if transfer.message.is_none() || transfer.received != transfer.total {
            return None;
        }
        let transfer = self.transfers.remove(key)?;
        let data: Vec<u8> = transfer.chunks.into_iter().flatten().flatten().collect();
        let message = transfer.message?;
        let file = message.get("file")?;
        if data.len() as i64 != number_of(file.get("size")?) || sha256_hex(&data) != file.get("sha256")?.as_str()? {
            return None;
        }
        Some(Completed { space_id: space_id.to_owned(), message, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::encode_base64;
    use serde_json::json;

    fn chunk_message(data: &[u8], index: usize, total: usize) -> Value {
        json!({ "from": "peer-1", "transferId": "transfer-1", "index": index, "total": total, "data": encode_base64(data) })
    }

    #[test]
    fn a_file_in_pieces_is_put_back_together_whatever_the_order() {
        let data: Vec<u8> = (0..CHUNK_BYTES * 2 + 500).map(|value| (value % 250) as u8).collect();
        let pieces = split_file(&data);
        assert_eq!(pieces.len(), 3);
        let announce = json!({
            "from": "peer-1", "messageId": "m1",
            "file": { "transferId": "transfer-1", "name": "a.bin", "size": data.len(), "sha256": sha256_hex(&data) },
        });

        let mut inbox = TransferInbox::default();
        assert!(inbox.add_chunk("space", &chunk_message(pieces[2], 2, 3)).is_none());
        let (accepted, done) = inbox.add_message("space", &announce);
        assert!(accepted && done.is_none());
        assert!(inbox.add_chunk("space", &chunk_message(pieces[0], 0, 3)).is_none());
        assert!(inbox.add_chunk("space", &chunk_message(pieces[0], 0, 3)).is_none());
        let done = inbox.add_chunk("space", &chunk_message(pieces[1], 1, 3)).unwrap();
        assert_eq!(done.data, data);
        assert_eq!(done.message["messageId"], "m1");
    }

    #[test]
    fn a_file_that_arrives_different_from_what_was_announced_is_dropped() {
        let mut inbox = TransferInbox::default();
        let announce = json!({
            "from": "peer-1",
            "file": { "transferId": "transfer-1", "size": 3, "sha256": sha256_hex(b"abc") },
        });
        assert!(inbox.add_message("space", &announce).0);
        assert!(inbox.add_chunk("space", &chunk_message(b"abd", 0, 1)).is_none());
        assert!(!inbox.add_message("space", &json!({ "from": "p", "file": { "transferId": "x" } })).0);
        assert!(!inbox.add_message("space", &json!({ "from": "p", "text": "sem arquivo" })).0);
    }

    #[test]
    fn file_names_lose_folders_and_what_windows_refuses() {
        assert_eq!(clean_file_name("C:\\pasta\\foto final.png"), "foto final.png");
        assert_eq!(clean_file_name("../../etc/passwd"), "passwd");
        assert_eq!(clean_file_name("a<b>:c|d?.txt"), "a_b__c_d_.txt");
        assert_eq!(clean_file_name("  ..oculto "), "oculto");
        assert_eq!(clean_file_name(""), "arquivo");
        assert_eq!(clean_mime(Some("image/gif")), "image/gif");
        assert_eq!(clean_mime(Some("not a mime")), "application/octet-stream");
        assert_eq!(clean_mime(None), "application/octet-stream");
    }
}
