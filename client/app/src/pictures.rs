//! The avatars and banners shown in the window, read from the hub once and kept until the person
//! changes their picture.

use std::collections::HashMap;
use std::sync::Arc;

use telinha_core::avatars::Picture;
use telinha_core::hub::Hub;

#[derive(Clone)]
struct Entry {
    uri: String,
    bytes: Arc<[u8]>,
}

fn extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        _ => "webp",
    }
}

impl Entry {
    fn new(kind: &str, picture: Picture) -> Self {
        Self {
            uri: format!("bytes://{kind}/{}.{}", picture.hash, extension(&picture.mime)),
            bytes: picture.data.into(),
        }
    }

    fn image(&self) -> egui::Image<'static> {
        egui::Image::from_bytes(self.uri.clone(), egui::load::Bytes::Shared(Arc::clone(&self.bytes)))
    }
}

#[derive(Default)]
pub struct Pictures {
    /// By member and kind. Nothing stored means the member was looked up and has no picture.
    known: HashMap<(String, bool), Option<Entry>>,
    own_member: String,
}

impl Pictures {
    pub fn new(hub: &Hub) -> Self {
        Self { known: HashMap::new(), own_member: hub.member_id() }
    }

    fn entry(&mut self, hub: &Hub, member_id: &str, banner: bool) -> Option<&Entry> {
        let key = (member_id.to_owned(), banner);
        if !self.known.contains_key(&key) {
            let picture = if member_id == self.own_member { hub.own_picture(banner) } else { hub.picture(member_id, banner) };
            let kind = if banner { "banner" } else { "avatar" };
            self.known.insert(key.clone(), picture.map(|picture| Entry::new(kind, picture)));
        }
        self.known.get(&key).and_then(Option::as_ref)
    }

    pub fn avatar(&mut self, hub: &Hub, member_id: Option<&str>) -> Option<egui::Image<'static>> {
        self.entry(hub, member_id?, false).map(Entry::image)
    }

    pub fn banner(&mut self, hub: &Hub, member_id: Option<&str>) -> Option<egui::Image<'static>> {
        self.entry(hub, member_id?, true).map(Entry::image)
    }

    /// Forgets what was read for a member, so the next draw reads the new picture.
    pub fn forget(&mut self, member_id: &str, banner: bool) {
        self.known.remove(&(member_id.to_owned(), banner));
    }

    pub fn forget_own(&mut self) {
        let member = self.own_member.clone();
        self.forget(&member, false);
        self.forget(&member, true);
    }
}
