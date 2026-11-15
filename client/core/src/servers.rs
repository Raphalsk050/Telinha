//! Servers: a group of people with channels, all sharing one secret that travels in the invite.
//! Nobody hosts a server. Its name, channels and relay live in every member's app, each with a
//! version, and the newest one wins when two members meet. Same file as `desktop/src/servers.js`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use indexmap::IndexMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::disk::{lenient_i64, number_of, read_json, write_json};
use crate::envelope::{SpaceKeys, space_keys};
use crate::text::{
    clean_name, decode_base64, encode_base64, encode_base64_url, now_ms, random_bytes, random_hex, truncate_utf16,
};
use crate::vault::Vault;

pub const INVITE_PREFIX: &str = "TELINHA-GRUPO.";
const MAX_CHANNELS: usize = 50;
const MAX_MEMBERS: usize = 500;
const MAX_CHANNEL_NAME: usize = 32;
const MAX_RELAY_URLS: usize = 4;
const MAX_RELAY_TEXT: usize = 128;

static INVITE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"TELINHA-GRUPO\.([A-Za-z0-9_-]+)").unwrap());
static CHANNEL_ID_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-f0-9]{16}$").unwrap());
static MEMBER_ID_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-f0-9]{32}$").unwrap());
static RELAY_URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^turns?:[^\s,]{1,200}$").unwrap());

pub fn is_channel_id(text: &str) -> bool {
    CHANNEL_ID_PATTERN.is_match(text)
}

pub fn is_member_id(text: &str) -> bool {
    MEMBER_ID_PATTERN.is_match(text)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelKind {
    Text,
    Voice,
}

impl ChannelKind {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "text" => Some(Self::Text),
            "voice" => Some(Self::Voice),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub kind: ChannelKind,
    pub name: String,
}

/// The TURN relay of a server: where calls and streams go when two people cannot connect
/// directly. More than one address, because many routers do not send back in what leaves for
/// their own public address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relay {
    pub urls: Vec<String>,
    pub username: String,
    pub credential: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub last_seen_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredServer {
    id: String,
    name: String,
    name_version: i64,
    secret: String,
    created_at: i64,
    channels: Vec<Channel>,
    channels_version: i64,
    relay: Option<Relay>,
    relay_version: i64,
    members: IndexMap<String, Member>,
}

/// A server as the rest of the app sees it: everything but the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub channels: Vec<Channel>,
    pub relay: Option<Relay>,
    pub members: Vec<ServerMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerMember {
    pub member_id: String,
    pub name: String,
    pub last_seen_at: i64,
}

impl From<&StoredServer> for Server {
    fn from(stored: &StoredServer) -> Self {
        Self {
            id: stored.id.clone(),
            name: stored.name.clone(),
            created_at: stored.created_at,
            channels: stored.channels.clone(),
            relay: stored.relay.clone(),
            members: stored
                .members
                .iter()
                .map(|(member_id, member)| ServerMember {
                    member_id: member_id.clone(),
                    name: member.name.clone(),
                    last_seen_at: member.last_seen_at,
                })
                .collect(),
        }
    }
}

/// Who is creating or joining, to be written as the first member.
#[derive(Debug, Clone, Copy)]
pub struct SelfMember<'a> {
    pub member_id: &'a str,
    pub name: &'a str,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MetaResult {
    /// Something here was replaced by a newer version from the other side.
    pub changed: bool,
    /// The other side is behind in something and should hear ours.
    pub mine_newer: bool,
}

fn new_channel_id() -> String {
    random_hex(8)
}

fn clean_channel_name(name: &str, kind: ChannelKind, fallback: &str) -> String {
    let separator = if kind == ChannelKind::Text { "-" } else { " " };
    let collapsed = name.split_whitespace().collect::<Vec<_>>().join(separator);
    let cased = if kind == ChannelKind::Text { collapsed.to_lowercase() } else { collapsed };
    let clean = truncate_utf16(&cased, MAX_CHANNEL_NAME);
    if clean.is_empty() { fallback.to_owned() } else { clean.to_owned() }
}

fn default_channel_name(kind: ChannelKind) -> &'static str {
    if kind == ChannelKind::Text { "canal" } else { "Sala" }
}

/// The channels of a list that are well formed, or nothing when it is not a list at all.
fn sanitize_channels(channels: Option<&Value>) -> Option<Vec<Channel>> {
    let list = channels?.as_array()?;
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for channel in list.iter().take(MAX_CHANNELS) {
        let Some(id) = channel.get("id").and_then(Value::as_str) else { continue };
        let Some(kind) = channel.get("kind").and_then(Value::as_str).and_then(ChannelKind::parse) else {
            continue;
        };
        if !is_channel_id(id) || !seen.insert(id.to_owned()) {
            continue;
        }
        let name = channel.get("name").and_then(Value::as_str).unwrap_or("");
        result.push(Channel { id: id.to_owned(), kind, name: clean_channel_name(name, kind, default_channel_name(kind)) });
    }
    Some(result)
}

/// The relay of a server as it may be stored and used, or nothing when there is no usable address.
pub fn sanitize_relay(relay: Option<&Value>) -> Option<Relay> {
    let relay = relay?.as_object()?;
    let list: Vec<String> = match relay.get("urls") {
        Some(Value::Array(urls)) => urls
            .iter()
            .map(|url| match url {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .collect(),
        Some(Value::String(text)) => text.split(',').map(str::to_owned).collect(),
        _ => Vec::new(),
    };
    let urls: Vec<String> = list
        .iter()
        .map(|url| url.trim().to_owned())
        .filter(|url| RELAY_URL_PATTERN.is_match(url))
        .take(MAX_RELAY_URLS)
        .collect();
    if urls.is_empty() {
        return None;
    }
    let field = |name: &str| {
        relay.get(name).and_then(Value::as_str).map_or_else(String::new, |text| truncate_utf16(text, MAX_RELAY_TEXT).to_owned())
    };
    Some(Relay { urls, username: field("username"), credential: field("credential") })
}

fn next_version(current: i64) -> i64 {
    now_ms().max(current + 1)
}

pub fn encode_invite(secret: &[u8], name: &str) -> String {
    let body = json!({ "v": 1, "s": encode_base64(secret), "n": name });
    format!("{INVITE_PREFIX}{}", encode_base64_url(body.to_string().as_bytes()))
}

/// The secret and the name inside an invite, found anywhere in a pasted text.
pub fn decode_invite(text: &str) -> Option<(Vec<u8>, String)> {
    let found = INVITE_PATTERN.captures(text)?;
    let data: Value = serde_json::from_slice(&decode_base64(&found[1])).ok()?;
    let secret = decode_base64(data.get("s")?.as_str()?);
    if data.get("v")?.as_i64()? != 1 || secret.len() != 32 {
        return None;
    }
    let name = data.get("n").and_then(Value::as_str).unwrap_or("");
    Some((secret, clean_name(name, "Servidor")))
}

/// The first invite code in a text, as written.
pub fn find_invite(text: &str) -> Option<&str> {
    INVITE_PATTERN.find(text).map(|found| found.as_str())
}

pub struct ServerStore {
    file: PathBuf,
    vault: Arc<dyn Vault>,
    servers: Vec<StoredServer>,
    keys: HashMap<String, SpaceKeys>,
}

impl ServerStore {
    pub fn load(file: impl Into<PathBuf>, vault: Arc<dyn Vault>) -> Self {
        let file = file.into();
        let data = read_json(&file);
        let servers = data
            .as_ref()
            .and_then(|data| data.get("servers"))
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(read_server).collect())
            .unwrap_or_default();
        Self { file, vault, servers, keys: HashMap::new() }
    }

    fn save(&self) {
        let _ = write_json(&self.file, &json!({ "version": 1, "servers": self.servers }));
    }

    pub fn list(&self) -> Vec<Server> {
        self.servers.iter().map(Server::from).collect()
    }

    pub fn get(&self, id: &str) -> Option<Server> {
        self.stored(id).map(Server::from)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.stored(id).is_some()
    }

    fn stored(&self, id: &str) -> Option<&StoredServer> {
        self.servers.iter().find(|server| server.id == id)
    }

    fn stored_mut(&mut self, id: &str) -> Option<&mut StoredServer> {
        self.servers.iter_mut().find(|server| server.id == id)
    }

    fn secret_for(&self, server: &StoredServer) -> Option<Vec<u8>> {
        let secret = decode_base64(&self.vault.decrypt(&server.secret)?);
        (!secret.is_empty()).then_some(secret)
    }

    pub fn keys_for(&mut self, id: &str) -> Option<SpaceKeys> {
        if let Some(keys) = self.keys.get(id) {
            return Some(keys.clone());
        }
        let keys = space_keys(&self.secret_for(self.stored(id)?)?);
        self.keys.insert(id.to_owned(), keys.clone());
        Some(keys)
    }

    /// What this app announces about a server, for the others to compare versions with.
    pub fn meta(&self, id: &str) -> Option<Value> {
        let server = self.stored(id)?;
        Some(json!({
            "name": server.name,
            "nameVersion": server.name_version,
            "channels": server.channels,
            "channelsVersion": server.channels_version,
            "relay": server.relay,
            "relayVersion": server.relay_version,
        }))
    }

    fn add_server(
        &mut self,
        secret: &[u8],
        name: &str,
        name_version: i64,
        channels: Vec<Channel>,
        channels_version: i64,
        me: Option<SelfMember<'_>>,
    ) -> Server {
        let now = now_ms();
        let mut members = IndexMap::new();
        if let Some(me) = me.filter(|me| is_member_id(me.member_id)) {
            members.insert(me.member_id.to_owned(), Member { name: clean_name(me.name, "Você"), last_seen_at: now });
        }
        let server = StoredServer {
            id: space_keys(secret).id,
            name: clean_name(name, "Servidor"),
            name_version,
            secret: self.vault.encrypt(&encode_base64(secret)),
            created_at: now,
            channels,
            channels_version,
            relay: None,
            relay_version: 0,
            members,
        };
        let view = Server::from(&server);
        self.servers.push(server);
        self.save();
        view
    }

    pub fn create(&mut self, name: &str, me: Option<SelfMember<'_>>) -> Server {
        let now = now_ms();
        let channels = vec![
            Channel { id: new_channel_id(), kind: ChannelKind::Text, name: "geral".into() },
            Channel { id: new_channel_id(), kind: ChannelKind::Voice, name: "Geral".into() },
        ];
        let secret: [u8; 32] = random_bytes();
        self.add_server(&secret, &clean_name(name, "Meu servidor"), now, channels, now, me)
    }

    /// Joins the server of an invite. The second value says whether it is new here.
    pub fn join(&mut self, code: &str, me: Option<SelfMember<'_>>) -> Option<(Server, bool)> {
        let (secret, name) = decode_invite(code)?;
        if let Some(existing) = self.get(&space_keys(&secret).id) {
            return Some((existing, false));
        }
        Some((self.add_server(&secret, &name, 0, Vec::new(), 0, me), true))
    }

    pub fn invite_code(&self, id: &str) -> Option<String> {
        let server = self.stored(id)?;
        Some(encode_invite(&self.secret_for(server)?, &server.name))
    }

    pub fn rename(&mut self, id: &str, name: &str) -> bool {
        let clean = clean_name(name, "");
        let Some(server) = self.stored_mut(id) else { return false };
        if clean.is_empty() || clean == server.name {
            return false;
        }
        server.name = clean;
        server.name_version = next_version(server.name_version);
        self.save();
        true
    }

    pub fn set_relay(&mut self, id: &str, relay: Option<&Value>) -> bool {
        let clean = sanitize_relay(relay);
        let Some(server) = self.stored_mut(id) else { return false };
        if clean == server.relay {
            return false;
        }
        server.relay = clean;
        server.relay_version = next_version(server.relay_version);
        self.save();
        true
    }

    pub fn leave(&mut self, id: &str) -> Option<Server> {
        let view = self.get(id)?;
        self.servers.retain(|server| server.id != id);
        self.keys.remove(id);
        self.save();
        Some(view)
    }

    /// Takes what is newer in the meta another member announced.
    pub fn apply_meta(&mut self, id: &str, meta: &Value) -> MetaResult {
        let mut result = MetaResult::default();
        if !meta.is_object() {
            return result;
        }
        let Some(server) = self.stored_mut(id) else { return result };
        let version = |name: &str| meta.get(name).map_or(0, number_of);

        let channels_version = version("channelsVersion");
        if channels_version > server.channels_version {
            if let Some(channels) = sanitize_channels(meta.get("channels")) {
                server.channels = channels;
                server.channels_version = channels_version;
                result.changed = true;
            }
        } else if channels_version < server.channels_version {
            result.mine_newer = true;
        }

        let name_version = version("nameVersion");
        let name = meta.get("name").and_then(Value::as_str);
        if let (true, Some(name)) = (name_version > server.name_version, name) {
            server.name = clean_name(name, &server.name.clone());
            server.name_version = name_version;
            result.changed = true;
        } else if name_version < server.name_version {
            result.mine_newer = true;
        }

        // An app that does not know the relay sends version 0 and does not count as behind.
        let relay_version = version("relayVersion");
        if relay_version > server.relay_version {
            server.relay = sanitize_relay(meta.get("relay"));
            server.relay_version = relay_version;
            result.changed = true;
        } else if relay_version > 0 && relay_version < server.relay_version {
            result.mine_newer = true;
        }

        if result.changed {
            self.save();
        }
        result
    }

    /// Notes that a member was seen. True when the member is new or changed name.
    pub fn touch_member(&mut self, id: &str, member_id: &str, name: &str) -> bool {
        if !is_member_id(member_id) {
            return false;
        }
        let clean = clean_name(name, "Alguém");
        let now = now_ms();
        let Some(server) = self.stored_mut(id) else { return false };
        let known = server.members.contains_key(member_id);
        if let Some(existing) = server.members.get_mut(member_id).filter(|existing| existing.name == clean) {
            existing.last_seen_at = now;
            return false;
        }
        if !known && server.members.len() >= MAX_MEMBERS {
            return false;
        }
        server.members.insert(member_id.to_owned(), Member { name: clean, last_seen_at: now });
        self.save();
        true
    }

    pub fn add_channel(&mut self, id: &str, kind: ChannelKind, name: &str) -> Option<Channel> {
        let server = self.stored_mut(id)?;
        if server.channels.len() >= MAX_CHANNELS {
            return None;
        }
        let fallback = if kind == ChannelKind::Text { "novo-canal" } else { "Nova sala" };
        let channel = Channel { id: new_channel_id(), kind, name: clean_channel_name(name, kind, fallback) };
        server.channels.push(channel.clone());
        server.channels_version = next_version(server.channels_version);
        self.save();
        Some(channel)
    }

    pub fn rename_channel(&mut self, id: &str, channel_id: &str, name: &str) -> bool {
        let Some(server) = self.stored_mut(id) else { return false };
        let Some(channel) = server.channels.iter_mut().find(|channel| channel.id == channel_id) else {
            return false;
        };
        let clean = clean_channel_name(name, channel.kind, &channel.name.clone());
        if clean == channel.name {
            return false;
        }
        channel.name = clean;
        server.channels_version = next_version(server.channels_version);
        self.save();
        true
    }

    pub fn remove_channel(&mut self, id: &str, channel_id: &str) -> bool {
        let Some(server) = self.stored_mut(id) else { return false };
        if !server.channels.iter().any(|channel| channel.id == channel_id) {
            return false;
        }
        server.channels.retain(|channel| channel.id != channel_id);
        server.channels_version = next_version(server.channels_version);
        self.save();
        true
    }
}

fn read_server(entry: &Value) -> Option<StoredServer> {
    let text = |name: &str| entry.get(name).and_then(Value::as_str).filter(|text| !text.is_empty());
    let number = |name: &str| entry.get(name).map_or(0, number_of);
    let members = entry
        .get("members")
        .and_then(Value::as_object)
        .map(read_members)
        .unwrap_or_default();
    let created_at = number("createdAt");
    Some(StoredServer {
        id: text("id")?.to_owned(),
        name: clean_name(text("name").unwrap_or(""), "Servidor"),
        name_version: number("nameVersion"),
        secret: text("secret")?.to_owned(),
        created_at: if created_at == 0 { now_ms() } else { created_at },
        channels: sanitize_channels(entry.get("channels")).unwrap_or_default(),
        channels_version: number("channelsVersion"),
        relay: sanitize_relay(entry.get("relay")),
        relay_version: number("relayVersion"),
        members,
    })
}

fn read_members(members: &Map<String, Value>) -> IndexMap<String, Member> {
    members
        .iter()
        .filter_map(|(member_id, member)| {
            Some((member_id.clone(), serde_json::from_value::<Member>(member.clone()).ok()?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::PlainVault;

    fn fresh_store() -> (tempfile::TempDir, ServerStore) {
        let folder = tempfile::tempdir().unwrap();
        let store = ServerStore::load(folder.path().join("servers.json"), Arc::new(PlainVault));
        (folder, store)
    }

    #[test]
    fn a_relay_keeps_turn_addresses_and_drops_anything_else() {
        let relay = sanitize_relay(Some(&json!({ "urls": "turn:a.example:3478", "username": "u", "credential": "p" })));
        assert_eq!(
            relay,
            Some(Relay { urls: vec!["turn:a.example:3478".into()], username: "u".into(), credential: "p".into() })
        );
        let relay = sanitize_relay(Some(&json!({ "urls": "turn:a.example:3478, https://x,turn:192.168.1.9:3478" })));
        assert_eq!(relay.unwrap().urls, ["turn:a.example:3478", "turn:192.168.1.9:3478"]);
        assert_eq!(sanitize_relay(Some(&json!({ "urls": "https://a.example" }))), None);
        assert_eq!(sanitize_relay(Some(&Value::Null)), None);
        assert_eq!(sanitize_relay(None), None);
    }

    #[test]
    fn the_relay_set_by_one_member_reaches_another_through_the_meta() {
        let (_owner_folder, mut owner) = fresh_store();
        let (_member_folder, mut member) = fresh_store();
        let id = owner.create("Teste", None).id;
        member.join(&owner.invite_code(&id).unwrap(), None).unwrap();

        let relay = json!({ "urls": ["turn:a.example:3478"], "username": "u", "credential": "p" });
        assert!(owner.set_relay(&id, Some(&relay)));
        assert!(!owner.set_relay(&id, Some(&relay)));
        assert!(member.apply_meta(&id, &owner.meta(&id).unwrap()).changed);
        assert_eq!(member.get(&id).unwrap().relay, sanitize_relay(Some(&relay)));

        // A member on an older version sends no relay and must not wipe it.
        let mut old = owner.meta(&id).unwrap();
        old.as_object_mut().unwrap().remove("relay");
        old.as_object_mut().unwrap().remove("relayVersion");
        assert!(!member.apply_meta(&id, &old).mine_newer);
        assert_eq!(member.get(&id).unwrap().relay, sanitize_relay(Some(&relay)));

        owner.set_relay(&id, None);
        member.apply_meta(&id, &owner.meta(&id).unwrap());
        assert_eq!(member.get(&id).unwrap().relay, None);
    }

    #[test]
    fn someone_who_joins_gets_the_name_and_channels_from_whoever_is_there() {
        let (_owner_folder, mut owner) = fresh_store();
        let (_member_folder, mut member) = fresh_store();
        let me = SelfMember { member_id: &"a".repeat(32), name: "Rafa" };
        let server = owner.create("  Meu   grupo ", Some(me));
        assert_eq!(server.name, "Meu grupo");
        assert_eq!(server.channels.len(), 2);
        assert_eq!(server.members[0].name, "Rafa");

        let (joined, created) = member.join(&format!("entra ai: {}", owner.invite_code(&server.id).unwrap()), None).unwrap();
        assert!(created);
        assert_eq!(joined.id, server.id);
        assert_eq!(joined.name, "Meu grupo");
        assert!(joined.channels.is_empty());
        assert!(!member.join(&owner.invite_code(&server.id).unwrap(), None).unwrap().1);

        let result = member.apply_meta(&server.id, &owner.meta(&server.id).unwrap());
        assert_eq!(result, MetaResult { changed: true, mine_newer: false });
        assert_eq!(member.get(&server.id).unwrap().channels, server.channels);
        assert_eq!(member.keys_for(&server.id), owner.keys_for(&server.id));
        assert!(member.join("TELINHA-GRUPO.bm9wZQ", None).is_none());
    }

    #[test]
    fn channels_are_named_like_the_kind_asks_and_each_change_raises_the_version() {
        let (_folder, mut store) = fresh_store();
        let id = store.create("Teste", None).id;
        let text = store.add_channel(&id, ChannelKind::Text, "  Canal  Dos Memes ").unwrap();
        assert_eq!(text.name, "canal-dos-memes");
        let voice = store.add_channel(&id, ChannelKind::Voice, "  Sala   de Jogo ").unwrap();
        assert_eq!(voice.name, "Sala de Jogo");
        assert_eq!(store.add_channel(&id, ChannelKind::Text, "   ").unwrap().name, "novo-canal");

        let before = store.meta(&id).unwrap()["channelsVersion"].as_i64().unwrap();
        assert!(store.rename_channel(&id, &text.id, "Outro Nome"));
        assert!(!store.rename_channel(&id, &text.id, "outro   nome"));
        assert!(store.remove_channel(&id, &voice.id));
        assert!(!store.remove_channel(&id, &voice.id));
        assert!(store.meta(&id).unwrap()["channelsVersion"].as_i64().unwrap() > before);
    }

    #[test]
    fn a_member_is_noted_once_and_again_only_when_the_name_changes() {
        let (folder, mut store) = fresh_store();
        let id = store.create("Teste", None).id;
        let member = "b".repeat(32);
        assert!(store.touch_member(&id, &member, "Keep"));
        assert!(!store.touch_member(&id, &member, "Keep"));
        assert!(store.touch_member(&id, &member, "Keep 2"));
        assert!(!store.touch_member(&id, "not-an-id", "x"));

        let reloaded = ServerStore::load(folder.path().join("servers.json"), Arc::new(PlainVault));
        assert_eq!(reloaded.get(&id).unwrap().members[0].name, "Keep 2");
        assert_eq!(reloaded.get(&id).unwrap().channels.len(), 2);
    }

    #[test]
    fn an_invite_from_the_javascript_app_is_read() {
        // encodeInvite(Buffer.alloc(32, 7), 'Servidor de rapha') in desktop/src/servers.js.
        let code = encode_invite(&[7u8; 32], "Servidor de rapha");
        assert!(code.starts_with(INVITE_PREFIX));
        assert!(!code.contains('='));
        let (secret, name) = decode_invite(&code).unwrap();
        assert_eq!(secret, vec![7u8; 32]);
        assert_eq!(name, "Servidor de rapha");
    }
}
