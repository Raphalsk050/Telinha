//! The sounds of the soundboard, kept as files next to an index. Same folder as
//! `desktop/src/sounds.js` wrote.

use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::disk::{lenient_i64, read_json, write_json};
use crate::text::{collapse_spaces, now_ms, random_hex, truncate_utf16};

pub const MAX_SOUND_BYTES: usize = 1024 * 1024;
pub const MAX_SOUNDS: usize = 60;

static SOUND_ID_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-f0-9]{16}$").unwrap());
static MIME_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^audio/[\w.+-]+$").unwrap());

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sound {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub mime: String,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub size: i64,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub created_at: i64,
}

fn clean_sound_name(name: &str) -> String {
    let clean = collapse_spaces(name, 32);
    if clean.is_empty() { "Som".to_owned() } else { clean }
}

pub struct SoundStore {
    directory: PathBuf,
    sounds: Vec<Sound>,
}

impl SoundStore {
    pub fn load(directory: impl Into<PathBuf>) -> Self {
        let directory = directory.into();
        let data = read_json(&directory.join("sounds.json"));
        let sounds = data
            .as_ref()
            .and_then(|data| data.get("sounds"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| serde_json::from_value::<Sound>(entry.clone()).ok())
            .filter(|sound| SOUND_ID_PATTERN.is_match(&sound.id) && MIME_PATTERN.is_match(&sound.mime))
            .map(|sound| Sound { name: clean_sound_name(&sound.name), ..sound })
            .collect();
        Self { directory, sounds }
    }

    fn save(&self) {
        let _ = write_json(&self.directory.join("sounds.json"), &json!({ "version": 1, "sounds": self.sounds }));
    }

    fn file_for(&self, id: &str) -> PathBuf {
        self.directory.join(format!("{id}.bin"))
    }

    pub fn list(&self) -> &[Sound] {
        &self.sounds
    }

    /// Adds a sound. The error is a sentence ready to show to the person.
    pub fn add(&mut self, name: &str, mime: &str, data: &[u8]) -> Result<Sound, String> {
        if self.sounds.len() >= MAX_SOUNDS {
            return Err(format!("o limite é de {MAX_SOUNDS} sons"));
        }
        if !MIME_PATTERN.is_match(mime) {
            return Err("isso não parece um arquivo de áudio".into());
        }
        if data.is_empty() || data.len() > MAX_SOUND_BYTES {
            return Err("o som precisa ter no máximo 1 MB".into());
        }
        let sound = Sound {
            id: random_hex(8),
            name: clean_sound_name(name),
            mime: truncate_utf16(mime, 60).to_owned(),
            size: data.len() as i64,
            created_at: now_ms(),
        };
        fs::create_dir_all(&self.directory).map_err(|error| error.to_string())?;
        fs::write(self.file_for(&sound.id), data).map_err(|error| error.to_string())?;
        self.sounds.push(sound.clone());
        self.save();
        Ok(sound)
    }

    pub fn rename(&mut self, id: &str, name: &str) -> bool {
        let Some(sound) = self.sounds.iter_mut().find(|sound| sound.id == id) else {
            return false;
        };
        sound.name = clean_sound_name(name);
        self.save();
        true
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.sounds.len();
        self.sounds.retain(|sound| sound.id != id);
        if self.sounds.len() == before {
            return false;
        }
        let _ = fs::remove_file(self.file_for(id));
        self.save();
        true
    }

    pub fn read(&self, id: &str) -> Option<Vec<u8>> {
        self.sounds.iter().find(|sound| sound.id == id)?;
        fs::read(self.file_for(id)).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sound_is_kept_renamed_and_removed() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = SoundStore::load(folder.path());
        let sound = store.add("  Risada   alta ", "audio/mpeg", b"mp3 bytes").unwrap();
        assert_eq!(sound.name, "Risada alta");
        assert!(store.add("x", "video/mp4", b"abc").is_err());
        assert!(store.add("x", "audio/ogg", b"").is_err());

        let mut reloaded = SoundStore::load(folder.path());
        assert_eq!(reloaded.list(), [sound.clone()]);
        assert_eq!(reloaded.read(&sound.id).unwrap(), b"mp3 bytes");
        assert!(reloaded.rename(&sound.id, ""));
        assert_eq!(reloaded.list()[0].name, "Som");
        assert!(reloaded.remove(&sound.id));
        assert!(!reloaded.remove(&sound.id));
        assert!(reloaded.read(&sound.id).is_none());
    }
}
