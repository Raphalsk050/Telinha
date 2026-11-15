//! Keys of a space (a contact or a server) and the sealed envelope every message travels in.
//! Byte for byte the same as `desktop/src/contacts.js`, which the apps already installed speak.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::text::{decode_base64, encode_base64, random_bytes};

const ENVELOPE_VERSION: u8 = 1;
const IV_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
pub const MAX_CLOCK_SKEW_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, PartialEq, Eq)]
pub struct SpaceKeys {
    /// Names the space on the message servers. Derived from the secret, never the secret itself.
    pub id: String,
    pub key: [u8; 32],
}

impl std::fmt::Debug for SpaceKeys {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("SpaceKeys").field("id", &self.id).finish_non_exhaustive()
    }
}

/// Removes every whitespace, as codes are pasted with line breaks in the middle.
pub fn normalize_code(text: &str) -> String {
    text.chars().filter(|character| !character.is_whitespace()).collect()
}

pub fn derive_contact_secret(invite_code: &str, answer_code: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"telinha-contact-v1\n");
    hash.update(normalize_code(invite_code).as_bytes());
    hash.update(b"\n");
    hash.update(normalize_code(answer_code).as_bytes());
    hash.finalize().into()
}

pub fn space_keys(secret: &[u8]) -> SpaceKeys {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret).expect("HMAC takes a key of any size");
    mac.update(b"telinha-contact-id");
    let id = hex::encode(mac.finalize().into_bytes())[..32].to_owned();

    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(Some(&[]), secret)
        .expand(b"telinha-signaling-key", &mut key)
        .expect("32 bytes is a valid HKDF output size");
    SpaceKeys { id, key }
}

/// version (1 byte), nonce (12), tag (16), ciphertext, all in base64.
pub fn seal_message(key: &[u8; 32], message: &Value) -> String {
    let iv: [u8; IV_BYTES] = random_bytes();
    let cipher = Aes256Gcm::new_from_slice(key).expect("the key has 32 bytes");
    let text = serde_json::to_vec(message).expect("a JSON value always serializes");
    let sealed = cipher
        .encrypt(&Nonce::from(iv), text.as_slice())
        .expect("AES-GCM does not fail on a message of this size");
    let (body, tag) = sealed.split_at(sealed.len() - TAG_BYTES);

    let mut envelope = Vec::with_capacity(1 + IV_BYTES + sealed.len());
    envelope.push(ENVELOPE_VERSION);
    envelope.extend_from_slice(&iv);
    envelope.extend_from_slice(tag);
    envelope.extend_from_slice(body);
    encode_base64(&envelope)
}

/// The message, or nothing when it is not for this key, was tampered with, or its clock is more
/// than five minutes away from ours, which is how a replayed message is told apart.
pub fn open_message(key: &[u8; 32], payload: &str, now_ms: i64) -> Option<Value> {
    let data = decode_base64(payload);
    if data.len() <= 1 + IV_BYTES + TAG_BYTES || data[0] != ENVELOPE_VERSION {
        return None;
    }
    let iv = &data[1..1 + IV_BYTES];
    let tag = &data[1 + IV_BYTES..1 + IV_BYTES + TAG_BYTES];
    let body = &data[1 + IV_BYTES + TAG_BYTES..];

    let mut sealed = Vec::with_capacity(body.len() + TAG_BYTES);
    sealed.extend_from_slice(body);
    sealed.extend_from_slice(tag);
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let nonce = Nonce::try_from(iv).ok()?;
    let text = cipher.decrypt(&nonce, sealed.as_slice()).ok()?;

    let message: Value = serde_json::from_slice(&text).ok()?;
    if !message.get("type").is_some_and(Value::is_string) {
        return None;
    }
    let at = message.get("at").and_then(Value::as_f64).filter(|at| at.is_finite())?;
    if (now_ms as f64 - at).abs() > MAX_CLOCK_SKEW_MS as f64 {
        return None;
    }
    Some(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const INVITE: &str = "TELINHA1.VExTaW52aXRlLWNvZGUtZm9yLXRlc3Rz";
    const ANSWER: &str = "TELINHA1.VExTYW5zd2VyLWNvZGUtZm9yLXRlc3Rz";

    #[test]
    fn both_sides_reach_the_same_keys_however_the_codes_were_pasted() {
        let one = space_keys(&derive_contact_secret(INVITE, ANSWER));
        let pasted = format!(" {INVITE}\n");
        let split = format!("{}\n{}", &ANSWER[..10], &ANSWER[10..]);
        let other = space_keys(&derive_contact_secret(&pasted, &split));
        assert_eq!(one.id, other.id);
        assert_eq!(one.key, other.key);
        assert_eq!(one.id.len(), 32);
    }

    #[test]
    fn keys_match_the_javascript_app() {
        // Values printed by desktop/src/contacts.js for the same two codes.
        let keys = space_keys(&derive_contact_secret(INVITE, ANSWER));
        let expected = include_str!("../tests/fixtures/keys.json");
        let expected: Value = serde_json::from_str(expected).unwrap();
        assert_eq!(keys.id, expected["id"].as_str().unwrap());
        assert_eq!(hex::encode(keys.key), expected["key"].as_str().unwrap());
    }

    #[test]
    fn a_message_sealed_by_the_javascript_app_opens_here() {
        let fixture: Value = serde_json::from_str(include_str!("../tests/fixtures/keys.json")).unwrap();
        let key: [u8; 32] = hex::decode(fixture["key"].as_str().unwrap()).unwrap().try_into().unwrap();
        let at = fixture["sealedAt"].as_i64().unwrap();
        let message = open_message(&key, fixture["sealed"].as_str().unwrap(), at).unwrap();
        assert_eq!(message["type"], "chat");
        assert_eq!(message["text"], "olá, açúcar 😀");
    }

    #[test]
    fn a_sealed_message_opens_only_with_its_key_and_near_its_time() {
        let key = [7u8; 32];
        let sealed = seal_message(&key, &json!({ "type": "chat", "at": 1_000_000, "text": "oi" }));
        assert_eq!(open_message(&key, &sealed, 1_000_500).unwrap()["text"], "oi");
        assert!(open_message(&[8u8; 32], &sealed, 1_000_500).is_none());
        assert!(open_message(&key, &sealed, 1_000_000 + MAX_CLOCK_SKEW_MS + 1).is_none());
        assert!(open_message(&key, "not an envelope", 1_000_500).is_none());

        let mut tampered = decode_base64(&sealed);
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(open_message(&key, &encode_base64(&tampered), 1_000_500).is_none());
    }

    #[test]
    fn a_message_without_a_type_or_a_time_is_refused() {
        let key = [7u8; 32];
        assert!(open_message(&key, &seal_message(&key, &json!({ "at": 5 })), 5).is_none());
        assert!(open_message(&key, &seal_message(&key, &json!({ "type": "chat" })), 5).is_none());
    }
}
