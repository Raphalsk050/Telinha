//! Avatar and banner pictures: the check every picture goes through before it is kept or sent,
//! and the folder where the pictures of other people are stored. Same layout as
//! `desktop/src/avatars.js`: `index.json` plus one raw file per member.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::disk::{read_json, write_json};
use crate::servers::is_member_id;
use crate::text::{decode_base64, encode_base64};

pub const MAX_AVATAR_BYTES: usize = 200 * 1024;

/// A picture that passed the check: one of the formats the app shows, within the size limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub mime: String,
    pub data: Vec<u8>,
    /// Names the picture in the presence, so only who does not have it asks for it.
    pub hash: String,
}

pub fn is_webp(data: &[u8]) -> bool {
    data.len() > 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP"
}

pub fn is_png(data: &[u8]) -> bool {
    data.len() > 8 && data[..4] == [0x89, 0x50, 0x4e, 0x47]
}

pub fn is_jpeg(data: &[u8]) -> bool {
    data.len() > 3 && data[..3] == [0xff, 0xd8, 0xff]
}

pub fn is_gif(data: &[u8]) -> bool {
    data.len() > 6 && &data[..4] == b"GIF8"
}

fn signature_matches(mime: &str, data: &[u8]) -> Option<bool> {
    match mime {
        "image/webp" => Some(is_webp(data)),
        "image/png" => Some(is_png(data)),
        "image/jpeg" => Some(is_jpeg(data)),
        _ => None,
    }
}

pub fn check_picture(mime: &str, data: Vec<u8>) -> Option<Picture> {
    if data.is_empty() || data.len() > MAX_AVATAR_BYTES || !signature_matches(mime, &data)? {
        return None;
    }
    let hash = hex::encode(Sha256::digest(&data))[..32].to_owned();
    Some(Picture { mime: mime.to_owned(), data, hash })
}

pub fn decode_picture(mime: &str, base64: &str) -> Option<Picture> {
    check_picture(mime, decode_base64(base64))
}

pub fn data_url(mime: &str, data: &[u8]) -> String {
    format!("data:{mime};base64,{}", encode_base64(data))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    mime: String,
    hash: String,
}

pub struct PictureStore {
    directory: PathBuf,
    members: IndexMap<String, Entry>,
    contacts: BTreeMap<String, String>,
}

impl PictureStore {
    pub fn load(directory: impl Into<PathBuf>) -> Self {
        let directory = directory.into();
        let data = read_json(&directory.join("index.json"));
        let mut members = IndexMap::new();
        let mut contacts = BTreeMap::new();
        if let Some(data) = &data {
            for (member_id, entry) in data.get("members").and_then(Value::as_object).into_iter().flatten() {
                let Ok(entry) = serde_json::from_value::<Entry>(entry.clone()) else { continue };
                if is_member_id(member_id) && signature_matches(&entry.mime, &[]).is_some() {
                    members.insert(member_id.clone(), entry);
                }
            }
            for (contact_id, member_id) in data.get("contacts").and_then(Value::as_object).into_iter().flatten() {
                if let Some(member_id) = member_id.as_str().filter(|member_id| is_member_id(member_id)) {
                    contacts.insert(contact_id.clone(), member_id.to_owned());
                }
            }
        }
        Self { directory, members, contacts }
    }

    fn save(&self) {
        let index = json!({ "version": 1, "members": self.members, "contacts": self.contacts });
        let _ = write_json(&self.directory.join("index.json"), &index);
    }

    fn file_for(&self, member_id: &str) -> PathBuf {
        self.directory.join(format!("{member_id}.bin"))
    }

    pub fn hash_for(&self, member_id: &str) -> &str {
        self.members.get(member_id).map_or("", |entry| entry.hash.as_str())
    }

    pub fn put(&mut self, member_id: &str, picture: &Picture) -> bool {
        if !is_member_id(member_id) || fs::create_dir_all(&self.directory).is_err() {
            return false;
        }
        if fs::write(self.file_for(member_id), &picture.data).is_err() {
            return false;
        }
        self.members.insert(member_id.to_owned(), Entry { mime: picture.mime.clone(), hash: picture.hash.clone() });
        self.save();
        true
    }

    pub fn remove(&mut self, member_id: &str) -> bool {
        if self.members.shift_remove(member_id).is_none() {
            return false;
        }
        let _ = fs::remove_file(self.file_for(member_id));
        self.save();
        true
    }

    /// The picture of a member, as stored.
    pub fn picture(&self, member_id: &str) -> Option<Picture> {
        let entry = self.members.get(member_id)?;
        let data = fs::read(self.file_for(member_id)).ok()?;
        Some(Picture { mime: entry.mime.clone(), data, hash: entry.hash.clone() })
    }

    /// Remembers which member is behind a contact, so the contact shows that member's picture.
    pub fn link_contact(&mut self, contact_id: &str, member_id: &str) -> bool {
        if !is_member_id(member_id) || self.contacts.get(contact_id).is_some_and(|known| known == member_id) {
            return false;
        }
        self.contacts.insert(contact_id.to_owned(), member_id.to_owned());
        self.save();
        true
    }

    pub fn member_of_contact(&self, contact_id: &str) -> Option<&str> {
        self.contacts.get(contact_id).map(String::as_str)
    }

    pub fn member_ids(&self) -> impl Iterator<Item = &str> {
        self.members.keys().map(String::as_str)
    }

    pub fn contact_links(&self) -> impl Iterator<Item = (&str, &str)> {
        self.contacts.iter().map(|(contact, member)| (contact.as_str(), member.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(text: &str) -> Picture {
        let mut data = vec![0xff, 0xd8, 0xff, 0xe0];
        data.extend_from_slice(text.as_bytes());
        check_picture("image/jpeg", data).unwrap()
    }

    #[test]
    fn only_a_known_format_within_the_limit_passes() {
        assert!(decode_picture("image/jpeg", &encode_base64(&[0xff, 0xd8, 0xff, 0xe0, 1])).is_some());
        assert!(decode_picture("image/png", &encode_base64(&[0xff, 0xd8, 0xff, 0xe0, 1])).is_none());
        assert!(decode_picture("image/gif", &encode_base64(b"GIF89a....")).is_none());
        assert!(decode_picture("image/jpeg", "").is_none());
        let mut large = vec![0xff, 0xd8, 0xff];
        large.resize(MAX_AVATAR_BYTES + 1, 0);
        assert!(check_picture("image/jpeg", large).is_none());
        assert_eq!(jpeg("a").hash.len(), 32);
        assert_ne!(jpeg("a").hash, jpeg("b").hash);
    }

    #[test]
    fn a_picture_is_kept_per_member_and_survives_a_reload() {
        let folder = tempfile::tempdir().unwrap();
        let member = "c".repeat(32);
        let mut store = PictureStore::load(folder.path());
        assert!(store.put(&member, &jpeg("foto")));
        assert!(!store.put("nope", &jpeg("foto")));
        assert!(store.link_contact("contato", &member));
        assert!(!store.link_contact("contato", &member));

        let mut reloaded = PictureStore::load(folder.path());
        assert_eq!(reloaded.hash_for(&member), jpeg("foto").hash);
        assert_eq!(reloaded.picture(&member).unwrap(), jpeg("foto"));
        assert_eq!(reloaded.member_of_contact("contato"), Some(member.as_str()));
        assert!(reloaded.remove(&member));
        assert!(!reloaded.remove(&member));
        assert!(reloaded.picture(&member).is_none());
    }
}
