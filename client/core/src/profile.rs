//! Who the person using this app is: the id the others know them by, the name, the pictures and
//! the style of the profile card. Same file as `desktop/src/profile.js` wrote.

use std::path::PathBuf;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::avatars::{Picture, decode_picture};
use crate::disk::{read_json, write_json};
use crate::servers::is_member_id;
use crate::text::{clean_name, encode_base64, random_hex, truncate_utf16};

const MAX_RECENT_AVATARS: usize = 6;
const MAX_BIO_LENGTH: usize = 190;
const MAX_BIO_LINES: usize = 6;
const MAX_PRONOUNS_LENGTH: usize = 40;

static HEX_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^#[0-9a-f]{6}$").unwrap());

/// How many colors each name effect takes, at least and at most.
fn effect_colors(effect: &str) -> Option<(usize, usize)> {
    match effect {
        "solid" | "neon" => Some((1, 1)),
        "gradient" => Some((2, 2)),
        "prism" => Some((2, 5)),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameStyle {
    pub effect: String,
    pub colors: Vec<String>,
}

/// The style of a profile: the effect on the name, the two colors of the card and the "about
/// me". The style of other people arrives in the presence, so what does not fit is dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Style {
    pub name: Option<NameStyle>,
    pub theme: Option<Vec<String>>,
    pub bio: String,
    /// How the person wants to be called, as "ela/dela", on one short line.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pronouns: String,
}

impl Style {
    /// Whether there is anything in it worth announcing.
    pub fn is_set(&self) -> bool {
        self.name.is_some() || self.theme.is_some() || !self.bio.is_empty() || !self.pronouns.is_empty()
    }
}

fn hex_colors(list: Option<&Value>, max: usize) -> Vec<String> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|color| HEX_PATTERN.is_match(color))
        .take(max)
        .map(str::to_lowercase)
        .collect()
}

fn clean_bio(value: Option<&Value>) -> String {
    let Some(text) = value.and_then(Value::as_str) else {
        return String::new();
    };
    let lines = text.replace('\r', "").split('\n').take(MAX_BIO_LINES).collect::<Vec<_>>().join("\n");
    truncate_utf16(&lines, MAX_BIO_LENGTH).trim().to_owned()
}

fn clean_pronouns(value: Option<&Value>) -> String {
    let text = value.and_then(Value::as_str).unwrap_or_default();
    let line = text.lines().next().unwrap_or_default();
    truncate_utf16(line.trim(), MAX_PRONOUNS_LENGTH).trim().to_owned()
}

pub fn clean_style(candidate: Option<&Value>) -> Style {
    let source = candidate.filter(|value| value.is_object());
    let wanted = source.and_then(|source| source.get("name")).filter(|value| value.is_object());
    let effect = wanted.and_then(|wanted| wanted.get("effect")).and_then(Value::as_str);
    let name = effect.and_then(|effect| {
        let (least, most) = effect_colors(effect)?;
        let colors = hex_colors(wanted.and_then(|wanted| wanted.get("colors")), most);
        (colors.len() >= least).then(|| NameStyle { effect: effect.to_owned(), colors })
    });
    let theme = hex_colors(source.and_then(|source| source.get("theme")), 2);
    Style {
        name,
        theme: (theme.len() == 2).then_some(theme),
        bio: clean_bio(source.and_then(|source| source.get("bio"))),
        pronouns: clean_pronouns(source.and_then(|source| source.get("pronouns"))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredPicture {
    mime: String,
    /// The picture in base64, as the file has always kept it.
    data: String,
    hash: String,
}

impl From<&Picture> for StoredPicture {
    fn from(picture: &Picture) -> Self {
        Self { mime: picture.mime.clone(), data: encode_base64(&picture.data), hash: picture.hash.clone() }
    }
}

impl StoredPicture {
    fn open(&self) -> Option<Picture> {
        decode_picture(&self.mime, &self.data)
    }
}

fn read_picture(value: Option<&Value>) -> Option<StoredPicture> {
    let stored: StoredPicture = serde_json::from_value(value?.clone()).ok()?;
    stored.open().as_ref().map(StoredPicture::from)
}

pub struct ProfileStore {
    file: PathBuf,
    member_id: String,
    name: String,
    avatar: Option<StoredPicture>,
    banner: Option<StoredPicture>,
    recent_avatars: Vec<StoredPicture>,
    style: Style,
}

impl ProfileStore {
    /// Reads the profile, creating the member id on the first run. `fallback_name` is used until
    /// the person chooses a name.
    pub fn load(file: impl Into<PathBuf>, fallback_name: &str) -> Self {
        let file = file.into();
        let data = read_json(&file);
        let field = |name: &str| data.as_ref().and_then(|data| data.get(name));
        let stored_id = field("memberId").and_then(Value::as_str).filter(|id| is_member_id(id));
        let fallback = clean_name(fallback_name, "Telinha");

        let store = Self {
            member_id: stored_id.map_or_else(|| random_hex(16), str::to_owned),
            name: clean_name(field("name").and_then(Value::as_str).unwrap_or(""), &fallback),
            avatar: read_picture(field("avatar")),
            banner: read_picture(field("banner")),
            recent_avatars: field("recentAvatars")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|entry| read_picture(Some(entry)))
                .take(MAX_RECENT_AVATARS)
                .collect(),
            style: clean_style(field("style")),
            file,
        };
        if stored_id.is_none() {
            store.save();
        }
        store
    }

    fn save(&self) {
        let profile = json!({
            "version": 1,
            "memberId": self.member_id,
            "name": self.name,
            "avatar": self.avatar,
            "banner": self.banner,
            "recentAvatars": self.recent_avatars,
            "style": self.style,
        });
        let _ = write_json(&self.file, &profile);
    }

    pub fn member_id(&self) -> &str {
        &self.member_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn style(&self) -> &Style {
        &self.style
    }

    pub fn avatar(&self) -> Option<Picture> {
        self.avatar.as_ref().and_then(StoredPicture::open)
    }

    pub fn banner(&self) -> Option<Picture> {
        self.banner.as_ref().and_then(StoredPicture::open)
    }

    pub fn avatar_hash(&self) -> &str {
        self.avatar.as_ref().map_or("", |avatar| avatar.hash.as_str())
    }

    pub fn banner_hash(&self) -> &str {
        self.banner.as_ref().map_or("", |banner| banner.hash.as_str())
    }

    /// The avatar or the banner in base64, ready to go in the answer to someone who asked for it.
    pub fn picture_base64(&self, banner: bool) -> Option<(&str, &str, &str)> {
        let picture = if banner { self.banner.as_ref() } else { self.avatar.as_ref() }?;
        Some((picture.mime.as_str(), picture.data.as_str(), picture.hash.as_str()))
    }

    pub fn recent_avatars(&self) -> Vec<Picture> {
        self.recent_avatars.iter().filter_map(StoredPicture::open).collect()
    }

    pub fn set_style(&mut self, style: Option<&Value>) -> bool {
        let clean = clean_style(style);
        if clean == self.style {
            return false;
        }
        self.style = clean;
        self.save();
        true
    }

    pub fn rename(&mut self, name: &str) -> bool {
        let clean = clean_name(name, &self.name);
        if clean == self.name {
            return false;
        }
        self.name = clean;
        self.save();
        true
    }

    pub fn set_avatar(&mut self, avatar: &Picture) {
        let stored = StoredPicture::from(avatar);
        self.recent_avatars.retain(|recent| recent.hash != stored.hash);
        self.recent_avatars.insert(0, stored.clone());
        self.recent_avatars.truncate(MAX_RECENT_AVATARS);
        self.avatar = Some(stored);
        self.save();
    }

    pub fn use_recent(&mut self, hash: &str) -> bool {
        let Some(position) = self.recent_avatars.iter().position(|recent| recent.hash == hash) else {
            return false;
        };
        let item = self.recent_avatars.remove(position);
        self.recent_avatars.insert(0, item.clone());
        self.avatar = Some(item);
        self.save();
        true
    }

    pub fn remove_avatar(&mut self) -> bool {
        if self.avatar.take().is_none() {
            return false;
        }
        self.save();
        true
    }

    pub fn set_banner(&mut self, banner: &Picture) {
        self.banner = Some(StoredPicture::from(banner));
        self.save();
    }

    pub fn remove_banner(&mut self) -> bool {
        if self.banner.take().is_none() {
            return false;
        }
        self.save();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avatars::check_picture;

    fn jpeg(text: &str) -> Picture {
        let mut data = vec![0xff, 0xd8, 0xff, 0xe0];
        data.extend_from_slice(text.as_bytes());
        check_picture("image/jpeg", data).unwrap()
    }

    fn name_of(style: Value) -> Option<NameStyle> {
        clean_style(Some(&style)).name
    }

    fn colors(list: &[&str]) -> Vec<String> {
        list.iter().map(|color| (*color).to_owned()).collect()
    }

    #[test]
    fn a_name_effect_keeps_the_number_of_colors_it_takes() {
        let solid = name_of(json!({ "name": { "effect": "solid", "colors": ["#FF0000", "#00ff00"] } })).unwrap();
        assert_eq!(solid, NameStyle { effect: "solid".into(), colors: colors(&["#ff0000"]) });
        let gradient =
            name_of(json!({ "name": { "effect": "gradient", "colors": ["#ff0000", "#00ff00", "#0000ff"] } })).unwrap();
        assert_eq!(gradient.colors, colors(&["#ff0000", "#00ff00"]));
        let prism =
            name_of(json!({ "name": { "effect": "prism", "colors": ["#111111", "red", "#222222", "#333333"] } })).unwrap();
        assert_eq!(prism.colors, colors(&["#111111", "#222222", "#333333"]));
        let many = name_of(json!({ "name": { "effect": "prism", "colors": vec!["#abcdef"; 9] } })).unwrap();
        assert_eq!(many.colors.len(), 5);
    }

    #[test]
    fn what_does_not_fit_the_format_is_dropped() {
        assert!(name_of(json!({ "name": { "effect": "gradient", "colors": ["#ff0000"] } })).is_none());
        assert!(name_of(json!({ "name": { "effect": "sparkle", "colors": ["#ff0000"] } })).is_none());
        assert!(name_of(json!({ "name": { "effect": "constructor", "colors": ["#ff0000"] } })).is_none());
        assert!(name_of(json!({ "name": "gradient" })).is_none());
        assert!(clean_style(Some(&json!({ "theme": ["#111111"] }))).theme.is_none());
        // The pronouns are one short line.
        let pronouns = clean_style(Some(&json!({ "pronouns": "  ela/dela\nmais nada  " }))).pronouns;
        assert_eq!(pronouns, "ela/dela");
        assert_eq!(clean_style(Some(&json!({ "pronouns": "x".repeat(80) }))).pronouns.len(), 40);
        assert!(clean_style(Some(&json!({ "theme": ["#111111", "url(x)"] }))).theme.is_none());
        let theme = clean_style(Some(&json!({ "theme": ["#111111", "#222222", "#333333"] }))).theme;
        assert_eq!(theme, Some(colors(&["#111111", "#222222"])));
        assert_eq!(clean_style(Some(&json!("nope"))), Style::default());
    }

    #[test]
    fn the_bio_is_cut_to_a_few_short_lines() {
        let bio = |value: Value| clean_style(Some(&json!({ "bio": value }))).bio;
        assert_eq!(bio(json!("  oi\r\ntudo bem  ")), "oi\ntudo bem");
        assert_eq!(bio(json!("x".repeat(500))).len(), 190);
        assert_eq!(bio(json!("a\nb\nc\nd\ne\nf\ng\nh")), "a\nb\nc\nd\ne\nf");
        assert_eq!(bio(json!(42)), "");
    }

    #[test]
    fn an_empty_style_is_told_from_one_worth_sending() {
        assert!(!clean_style(None).is_set());
        assert!(clean_style(Some(&json!({ "bio": "oi" }))).is_set());
        assert!(clean_style(Some(&json!({ "theme": ["#111111", "#222222"] }))).is_set());
    }

    #[test]
    fn the_style_is_written_like_the_javascript_app_announces_it() {
        let style = clean_style(Some(&json!({ "name": { "effect": "neon", "colors": ["#19B8B0"] }, "bio": "oi" })));
        assert_eq!(
            serde_json::to_string(&style).unwrap(),
            r##"{"name":{"effect":"neon","colors":["#19b8b0"]},"theme":null,"bio":"oi"}"##
        );
    }

    #[test]
    fn the_profile_keeps_its_style_and_banner_across_restarts() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("profile.json");
        let mut first = ProfileStore::load(&file, "Rafa");
        assert!(first.set_style(Some(&json!({ "name": { "effect": "neon", "colors": ["#19b8b0"] }, "bio": "oi" }))));
        assert!(!first.set_style(Some(&json!({ "name": { "effect": "neon", "colors": ["#19B8B0"] }, "bio": "oi " }))));
        let banner = jpeg("banner");
        first.set_banner(&banner);

        let mut again = ProfileStore::load(&file, "Rafa");
        assert_eq!(again.style().name.as_ref().unwrap().effect, "neon");
        assert_eq!(again.style().bio, "oi");
        assert_eq!(again.banner_hash(), banner.hash);
        assert_eq!(again.banner().unwrap(), banner);
        assert_eq!(again.member_id(), first.member_id());
        assert_eq!(again.name(), "Rafa");

        assert!(again.remove_banner());
        assert!(!again.remove_banner());
        assert!(ProfileStore::load(&file, "Rafa").banner().is_none());
    }

    #[test]
    fn the_last_avatars_are_kept_to_pick_again() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = ProfileStore::load(folder.path().join("profile.json"), "Rafa");
        for index in 0..8 {
            store.set_avatar(&jpeg(&format!("avatar {index}")));
        }
        assert_eq!(store.recent_avatars().len(), 6);
        assert_eq!(store.avatar_hash(), jpeg("avatar 7").hash);

        assert!(store.use_recent(&jpeg("avatar 3").hash));
        assert_eq!(store.avatar_hash(), jpeg("avatar 3").hash);
        assert_eq!(store.recent_avatars()[0], jpeg("avatar 3"));
        assert!(!store.use_recent("nope"));
        assert!(store.remove_avatar());
        assert_eq!(store.avatar_hash(), "");
        assert!(store.rename("  Novo   nome "));
        assert!(!store.rename("Novo nome"));
    }
}
