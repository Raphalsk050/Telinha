//! Protects what is written to disk with a key only this Windows account can open. The format is
//! the one Electron's `safeStorage` writes, so the data folder of the old app opens here and the
//! other way around: a key in `Local State` under DPAPI, and each value as `v10`, a nonce and
//! AES-256-GCM.

use std::fs;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use serde_json::{Map, Value};

use crate::text::{decode_base64, encode_base64, random_bytes};

const KEY_PREFIX: &[u8] = b"DPAPI";
const VALUE_PREFIX: &[u8] = b"v10";
const NONCE_BYTES: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("nao consegui ler ou gravar a chave do cofre: {0}")]
    Io(#[from] std::io::Error),
    #[error("o Windows recusou proteger ou abrir a chave do cofre")]
    System,
    #[error("a chave do cofre em Local State esta estragada")]
    BadKey,
}

/// Turns a text into what goes to disk and back. `decrypt` gives nothing for a value that was
/// not written by this account's vault.
pub trait Vault: Send + Sync {
    fn encrypt(&self, text: &str) -> String;
    fn decrypt(&self, stored: &str) -> Option<String>;
}

/// No protection at all, for tests and for a system where the real one is not available.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlainVault;

impl Vault for PlainVault {
    fn encrypt(&self, text: &str) -> String {
        text.to_owned()
    }

    fn decrypt(&self, stored: &str) -> Option<String> {
        Some(stored.to_owned())
    }
}

pub struct OsVault {
    key: [u8; 32],
}

impl OsVault {
    /// Opens the key kept in `data_dir`, creating it on the first run.
    pub fn open(data_dir: &Path) -> Result<Self, VaultError> {
        let file = data_dir.join("Local State");
        let mut state = read_state(&file);
        let stored = state
            .get("os_crypt")
            .and_then(|section| section.get("encrypted_key"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        if let Some(stored) = stored {
            let wrapped = decode_base64(&stored);
            let wrapped = wrapped.strip_prefix(KEY_PREFIX).ok_or(VaultError::BadKey)?;
            let key = dpapi::unprotect(wrapped).ok_or(VaultError::System)?;
            let key: [u8; 32] = key.try_into().map_err(|_| VaultError::BadKey)?;
            return Ok(Self { key });
        }

        let key: [u8; 32] = random_bytes();
        let mut wrapped = KEY_PREFIX.to_vec();
        wrapped.extend(dpapi::protect(&key).ok_or(VaultError::System)?);
        let section = state.entry("os_crypt").or_insert_with(|| Value::Object(Map::new()));
        if !section.is_object() {
            *section = Value::Object(Map::new());
        }
        section["encrypted_key"] = Value::String(encode_base64(&wrapped));
        write_state(&file, &state)?;
        Ok(Self { key })
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(&self.key).expect("the key has 32 bytes")
    }
}

impl Vault for OsVault {
    fn encrypt(&self, text: &str) -> String {
        let nonce: [u8; NONCE_BYTES] = random_bytes();
        let sealed = self
            .cipher()
            .encrypt(&Nonce::from(nonce), text.as_bytes())
            .expect("AES-GCM does not fail on a value of this size");
        let mut stored = VALUE_PREFIX.to_vec();
        stored.extend_from_slice(&nonce);
        stored.extend(sealed);
        encode_base64(&stored)
    }

    fn decrypt(&self, stored: &str) -> Option<String> {
        let data = decode_base64(stored);
        let Some(value) = data.strip_prefix(VALUE_PREFIX) else {
            // Written before the account had a key: the value itself is under DPAPI.
            return String::from_utf8(dpapi::unprotect(&data)?).ok();
        };
        if value.len() < NONCE_BYTES {
            return None;
        }
        let (nonce, sealed) = value.split_at(NONCE_BYTES);
        let nonce = Nonce::try_from(nonce).ok()?;
        String::from_utf8(self.cipher().decrypt(&nonce, sealed).ok()?).ok()
    }
}

fn read_state(file: &Path) -> Map<String, Value> {
    fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

fn write_state(file: &Path, state: &Map<String, Value>) -> Result<(), VaultError> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = temporary_path(file);
    fs::write(&temporary, serde_json::to_string(state).expect("a JSON map always serializes"))?;
    fs::rename(&temporary, file)?;
    Ok(())
}

pub(crate) fn temporary_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(windows)]
mod dpapi {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    fn take(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        if blob.pbData.is_null() {
            return Vec::new();
        }
        // SAFETY: the system filled the blob with `cbData` bytes it allocated with LocalAlloc.
        let data = unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }.to_vec();
        // SAFETY: the pointer came from the system call and is released exactly once.
        unsafe { LocalFree(Some(HLOCAL(blob.pbData.cast()))) };
        data
    }

    pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr().cast_mut() };
        let mut output = CRYPT_INTEGER_BLOB::default();
        // SAFETY: both blobs are valid for the call and the input is only read.
        unsafe {
            CryptProtectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        }
        .ok()?;
        Some(take(output))
    }

    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr().cast_mut() };
        let mut output = CRYPT_INTEGER_BLOB::default();
        // SAFETY: both blobs are valid for the call and the input is only read.
        unsafe {
            CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        }
        .ok()?;
        Some(take(output))
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn protect(_data: &[u8]) -> Option<Vec<u8>> {
        None
    }

    pub fn unprotect(_data: &[u8]) -> Option<Vec<u8>> {
        None
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn a_value_survives_closing_and_reopening_the_vault() {
        let folder = tempfile::tempdir().unwrap();
        let stored = OsVault::open(folder.path()).unwrap().encrypt("segredo com acento: ção");
        assert_ne!(stored, "segredo com acento: ção");

        let reopened = OsVault::open(folder.path()).unwrap();
        assert_eq!(reopened.decrypt(&stored).as_deref(), Some("segredo com acento: ção"));
        assert!(reopened.decrypt("AAAA").is_none());
    }

    #[test]
    fn the_key_file_keeps_what_was_already_in_it() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join("Local State"), r#"{"other":{"kept":true}}"#).unwrap();
        OsVault::open(folder.path()).unwrap();
        let state = read_state(&folder.path().join("Local State"));
        assert_eq!(state["other"]["kept"], true);
        assert!(state["os_crypt"]["encrypted_key"].is_string());
    }

    #[test]
    fn two_folders_do_not_open_each_others_values() {
        let one = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let stored = OsVault::open(one.path()).unwrap().encrypt("x");
        assert!(OsVault::open(other.path()).unwrap().decrypt(&stored).is_none());
    }

    /// Run by hand with `cargo test -p telinha-core real_data -- --ignored --nocapture`: opens the
    /// data the Electron app left on this machine, read only, and prints no secret.
    #[test]
    #[ignore]
    fn real_data_written_by_the_electron_app_opens() {
        let folder = PathBuf::from(std::env::var("APPDATA").unwrap()).join("Telinha");
        let vault = OsVault::open(&folder).unwrap();
        let servers: Value =
            serde_json::from_str(&fs::read_to_string(folder.join("servers.json")).unwrap()).unwrap();
        let list = servers["servers"].as_array().unwrap();
        assert!(!list.is_empty());
        for server in list {
            let secret = vault.decrypt(server["secret"].as_str().unwrap()).expect("the secret opens");
            let bytes = decode_base64(&secret);
            println!("server {} secret of {} bytes", server["id"], bytes.len());
            assert_eq!(bytes.len(), 32);
            assert_eq!(crate::envelope::space_keys(&bytes).id, server["id"].as_str().unwrap());
        }
    }
}
