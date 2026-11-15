//! People added one by one, each with a secret both sides derived from the two codes they
//! exchanged. Same file as `desktop/src/contacts.js` wrote.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::disk::{lenient_bool, lenient_i64, read_entries, read_json, write_json};
use crate::envelope::{SpaceKeys, derive_contact_secret, space_keys};
use crate::text::{clean_name, decode_base64, encode_base64, now_ms};
use crate::vault::Vault;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredContact {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default, deserialize_with = "lenient_bool")]
    name_customized: bool,
    #[serde(default, deserialize_with = "lenient_i64")]
    created_at: i64,
    #[serde(default, deserialize_with = "lenient_i64")]
    last_connected_at: i64,
    secret: String,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// What the rest of the app sees of a contact: everything but the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub last_connected_at: i64,
}

impl From<&StoredContact> for Contact {
    fn from(stored: &StoredContact) -> Self {
        Self {
            id: stored.id.clone(),
            name: stored.name.clone(),
            created_at: stored.created_at,
            last_connected_at: stored.last_connected_at,
        }
    }
}

pub struct ContactStore {
    file: PathBuf,
    vault: Arc<dyn Vault>,
    contacts: Vec<StoredContact>,
    keys: HashMap<String, SpaceKeys>,
}

impl ContactStore {
    pub fn load(file: impl Into<PathBuf>, vault: Arc<dyn Vault>) -> Self {
        let file = file.into();
        let data = read_json(&file);
        let contacts = read_entries::<StoredContact>(data.as_ref().and_then(|data| data.get("contacts")))
            .into_iter()
            .filter(|contact| !contact.id.is_empty() && !contact.secret.is_empty())
            .collect();
        Self { file, vault, contacts, keys: HashMap::new() }
    }

    fn save(&self) {
        // A full disk must not take the app down; the contact stays in memory for this run.
        let _ = write_json(&self.file, &json!({ "version": 1, "contacts": self.contacts }));
    }

    pub fn list(&self) -> Vec<Contact> {
        self.contacts.iter().map(Contact::from).collect()
    }

    pub fn get(&self, id: &str) -> Option<Contact> {
        self.contacts.iter().find(|contact| contact.id == id).map(Contact::from)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.contacts.iter().any(|contact| contact.id == id)
    }

    fn stored_mut(&mut self, id: &str) -> Option<&mut StoredContact> {
        self.contacts.iter_mut().find(|contact| contact.id == id)
    }

    pub fn keys_for(&mut self, id: &str) -> Option<SpaceKeys> {
        if let Some(keys) = self.keys.get(id) {
            return Some(keys.clone());
        }
        let contact = self.contacts.iter().find(|contact| contact.id == id)?;
        let secret = decode_base64(&self.vault.decrypt(&contact.secret)?);
        if secret.is_empty() {
            return None;
        }
        let keys = space_keys(&secret);
        self.keys.insert(id.to_owned(), keys.clone());
        Some(keys)
    }

    /// Saves the person behind a pair of codes. The same pair always gives the same contact.
    pub fn save_from_codes(&mut self, invite_code: &str, answer_code: &str, name: &str) -> (Contact, bool) {
        let secret = derive_contact_secret(invite_code, answer_code);
        let id = space_keys(&secret).id;
        let now = now_ms();

        let created = !self.contains(&id);
        if created {
            self.contacts.push(StoredContact {
                id: id.clone(),
                name: clean_name(name, "Contato"),
                name_customized: false,
                created_at: now,
                last_connected_at: now,
                secret: self.vault.encrypt(&encode_base64(&secret)),
                extra: Map::new(),
            });
        } else if let Some(contact) = self.stored_mut(&id) {
            contact.last_connected_at = now;
        }
        self.save();
        (self.get(&id).expect("the contact was just saved"), created)
    }

    pub fn rename(&mut self, id: &str, name: &str) -> bool {
        let Some(contact) = self.stored_mut(id) else {
            return false;
        };
        contact.name = clean_name(name, &contact.name.clone());
        contact.name_customized = true;
        self.save();
        true
    }

    /// Takes the name the other side announces, unless the person here chose one.
    pub fn adopt_remote_name(&mut self, id: &str, name: &str) -> bool {
        let clean = clean_name(name, "");
        let Some(contact) = self.stored_mut(id) else {
            return false;
        };
        if contact.name_customized || clean.is_empty() || clean == contact.name {
            return false;
        }
        contact.name = clean;
        self.save();
        true
    }

    pub fn touch(&mut self, id: &str) -> bool {
        let Some(contact) = self.stored_mut(id) else {
            return false;
        };
        contact.last_connected_at = now_ms();
        self.save();
        true
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.contacts.len();
        self.contacts.retain(|contact| contact.id != id);
        self.keys.remove(id);
        if self.contacts.len() == before {
            return false;
        }
        self.save();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::PlainVault;

    const INVITE: &str = "TELINHA1.VExTaW52aXRlLWNvZGUtZm9yLXRlc3Rz";
    const ANSWER: &str = "TELINHA1.VExTYW5zd2VyLWNvZGUtZm9yLXRlc3Rz";

    /// Reverses the text, enough to tell a protected secret from a plain one.
    struct Backwards;

    impl Vault for Backwards {
        fn encrypt(&self, text: &str) -> String {
            text.chars().rev().collect()
        }

        fn decrypt(&self, stored: &str) -> Option<String> {
            Some(stored.chars().rev().collect())
        }
    }

    #[test]
    fn a_contact_is_saved_once_with_its_secret_protected_and_survives_a_reload() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("contacts.json");
        let mut store = ContactStore::load(&file, Arc::new(Backwards));

        let (first, created) = store.save_from_codes(INVITE, ANSWER, "  Amigo   do  jogo ");
        assert!(created);
        assert_eq!(first.name, "Amigo do jogo");
        assert!(!store.save_from_codes(INVITE, ANSWER, "outro nome").1);
        assert_eq!(store.list().len(), 1);

        let raw = std::fs::read_to_string(&file).unwrap();
        let secret = derive_contact_secret(INVITE, ANSWER);
        assert!(!raw.contains(&encode_base64(&secret)));

        let mut reloaded = ContactStore::load(&file, Arc::new(Backwards));
        assert_eq!(reloaded.list()[0].id, first.id);
        assert_eq!(reloaded.keys_for(&first.id), Some(space_keys(&secret)));
    }

    #[test]
    fn a_name_chosen_here_wins_over_the_one_the_other_side_announces() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = ContactStore::load(folder.path().join("contacts.json"), Arc::new(PlainVault));
        let (contact, _) = store.save_from_codes(INVITE, ANSWER, "Contato");

        assert!(store.adopt_remote_name(&contact.id, "Rafa"));
        assert_eq!(store.get(&contact.id).unwrap().name, "Rafa");
        assert!(store.rename(&contact.id, "Irmao"));
        assert!(!store.adopt_remote_name(&contact.id, "Rafa"));
        assert_eq!(store.get(&contact.id).unwrap().name, "Irmao");

        assert!(store.remove(&contact.id));
        assert!(!store.remove(&contact.id));
        assert!(store.keys_for(&contact.id).is_none());
    }

    #[test]
    fn a_file_written_by_the_javascript_app_loads_and_keeps_fields_it_does_not_know() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("contacts.json");
        std::fs::write(
            &file,
            r#"{"version":1,"contacts":[
                {"id":"abc","name":"Keep","nameCustomized":true,"createdAt":1700000000000.0,
                 "lastConnectedAt":1700000000001,"secret":"c2VncmVkbw==","future":{"x":1}},
                {"id":"","secret":"x"}, {"name":"sem id"}, 7]}"#,
        )
        .unwrap();
        let mut store = ContactStore::load(&file, Arc::new(PlainVault));
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.list()[0].created_at, 1_700_000_000_000);

        assert!(store.touch("abc"));
        let raw = std::fs::read_to_string(&file).unwrap();
        assert!(raw.contains("\"future\""));
        assert!(raw.contains("\"nameCustomized\": true"));
    }
}
