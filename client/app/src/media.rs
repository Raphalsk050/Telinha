//! Media behind the links of the chat: GIFs that move, and videos that play inside the
//! conversation. The sender only sends a still of a link; whoever reads it fetches the moving
//! picture, as Discord shows a GIF of Tenor or Giphy playing.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use telinha_core::linkpreview::LinkPreviews;

/// Where the moving picture of a link is, when the link is of one: a GIF or WebP file, or a page
/// of Giphy, Tenor or Imgur, whose picture is a GIF.
pub fn animated_source(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let path = parsed.path().to_ascii_lowercase();
    // Imgur calls its short videos .gifv; the same name with .gif is the picture.
    if path.ends_with(".gifv") {
        return Some(url[..url.len() - parsed.query().map_or(0, |query| query.len() + 1) - 1].to_owned());
    }
    if path.ends_with(".gif") || path.ends_with(".webp") {
        return Some(url.to_owned());
    }
    let hosts = ["giphy.com", "tenor.com", "imgur.com", "gfycat.com"];
    hosts.iter().any(|known| host == *known || host.ends_with(&format!(".{known}"))).then(|| url.to_owned())
}

/// Whether the link is a page of a site of GIFs, whose page is only the picture.
pub fn gif_site(url: &str) -> bool {
    let host = url::Url::parse(url).ok().and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase)).unwrap_or_default();
    ["giphy.com", "tenor.com"].iter().any(|known| host == *known || host.ends_with(&format!(".{known}")))
}

/// The format of a picture that moves: "gif", or "webp" for a WebP with more than one frame.
pub fn animated_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"GIF8") {
        return Some("gif");
    }
    // A WebP that moves starts with the extended header, whose flags have the animation bit.
    if bytes.len() > 21 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" && &bytes[12..16] == b"VP8X" && bytes[20] & 0x02 != 0 {
        return Some("webp");
    }
    None
}

/// A moving picture fetched for a link.
#[derive(Clone)]
pub struct Animated {
    pub uri: String,
    pub bytes: Arc<[u8]>,
    pub size: egui::Vec2,
}

/// The moving pictures of the links shown, fetched once each.
pub struct AnimatedLinks {
    previews: Arc<LinkPreviews>,
    /// By the address fetched: None while it comes, and for good when it is not a moving picture.
    found: HashMap<String, Option<Animated>>,
    sender: Sender<(String, Option<Animated>)>,
    receiver: Receiver<(String, Option<Animated>)>,
}

impl Default for AnimatedLinks {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self { previews: Arc::new(LinkPreviews::default()), found: HashMap::new(), sender, receiver }
    }
}

impl AnimatedLinks {
    /// The moving picture of `source`, once it arrived. The first call starts fetching it.
    pub fn get(&mut self, context: &egui::Context, source: &str) -> Option<Animated> {
        while let Ok((address, animated)) = self.receiver.try_recv() {
            self.found.insert(address, animated);
        }
        if let Some(found) = self.found.get(source) {
            return found.clone();
        }
        self.found.insert(source.to_owned(), None);
        // The window draws inside the async runtime, which is where the fetch runs.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return None };
        let (previews, sender, wake, address) = (Arc::clone(&self.previews), self.sender.clone(), context.clone(), source.to_owned());
        runtime.spawn(async move {
            let image = previews.get(&address).await.and_then(|preview| preview.image);
            let animated = image.and_then(|image| {
                let format = animated_format(&image.data)?;
                let size = image::ImageReader::new(std::io::Cursor::new(&image.data)).with_guessed_format().ok()?.into_dimensions().ok()?;
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                address.hash(&mut hasher);
                let uri = format!("bytes://moving/{:016x}.{format}", hasher.finish());
                Some(Animated { uri, bytes: image.data.into(), size: egui::vec2(size.0.max(1) as f32, size.1.max(1) as f32) })
            });
            let _ = sender.send((address, animated));
            wake.request_repaint();
        });
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_of_moving_pictures_are_known_and_others_are_not() {
        assert_eq!(animated_source("https://media.tenor.com/abc/gato.gif").as_deref(), Some("https://media.tenor.com/abc/gato.gif"));
        assert_eq!(animated_source("https://tenor.com/view/gato-123").as_deref(), Some("https://tenor.com/view/gato-123"));
        assert_eq!(animated_source("https://giphy.com/gifs/abc").as_deref(), Some("https://giphy.com/gifs/abc"));
        assert_eq!(animated_source("https://i.imgur.com/abc.gifv").as_deref(), Some("https://i.imgur.com/abc.gif"));
        assert_eq!(animated_source("https://example.com/receita"), None);
        assert_eq!(animated_source("não é link"), None);
    }

    #[test]
    fn only_pictures_with_frames_count_as_moving() {
        assert_eq!(animated_format(b"GIF89a......"), Some("gif"));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\x02\0\0\0\0\0\0\0\0\0".to_vec();
        assert_eq!(animated_format(&webp), Some("webp"));
        webp[20] = 0;
        assert_eq!(animated_format(&webp), None);
        assert_eq!(animated_format(b"\x89PNG\r\n"), None);
    }
}
