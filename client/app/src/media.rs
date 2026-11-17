//! Media behind the links of the chat: GIFs that move, and videos that play inside the
//! conversation. The sender only sends a still of a link; whoever reads it fetches the moving
//! picture, as Discord shows a GIF of Tenor or Giphy playing.

use std::collections::{HashMap, HashSet};
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
    /// The addresses still on the way.
    loading: HashSet<String>,
    sender: Sender<(String, Option<Animated>)>,
    receiver: Receiver<(String, Option<Animated>)>,
}

impl Default for AnimatedLinks {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self { previews: Arc::new(LinkPreviews::default()), found: HashMap::new(), loading: HashSet::new(), sender, receiver }
    }
}

impl AnimatedLinks {
    /// Whether the moving picture of `source` was asked and has not arrived yet.
    pub fn loading(&self, source: &str) -> bool {
        self.loading.contains(source)
    }

    /// The moving picture of `source`, once it arrived. The first call starts fetching it.
    pub fn get(&mut self, context: &egui::Context, source: &str) -> Option<Animated> {
        while let Ok((address, animated)) = self.receiver.try_recv() {
            self.loading.remove(&address);
            self.found.insert(address, animated);
        }
        if let Some(found) = self.found.get(source) {
            return found.clone();
        }
        self.found.insert(source.to_owned(), None);
        // The window draws inside the async runtime, which is where the fetch runs.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return None };
        self.loading.insert(source.to_owned());
        let (previews, sender, wake, address) = (Arc::clone(&self.previews), self.sender.clone(), context.clone(), source.to_owned());
        runtime.spawn(async move {
            let image = previews.picture(&address).await;
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


/// A video a link leads to, and how it plays inside the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Video {
    /// The player of the site, put in a frame, or a file the browser plays itself.
    pub source: VideoSource,
    /// A video standing up, as Shorts and TikTok, shown 9 by 16.
    pub tall: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoSource {
    Player(String),
    File(String),
}

/// The video of a link, when it is one: YouTube, Vimeo, Streamable, TikTok, Dailymotion, a clip
/// of Twitch, or a video file.
pub fn video_of(address: &str) -> Option<Video> {
    let url = url::Url::parse(address).ok()?;
    if url.scheme() != "https" && url.scheme() != "http" {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.trim_start_matches("www.").trim_start_matches("m.").to_owned();
    let parts: Vec<&str> = url.path_segments().map(|parts| parts.filter(|part| !part.is_empty()).collect()).unwrap_or_default();
    let query = |name: &str| url.query_pairs().find(|(key, _)| key == name).map(|(_, value)| value.into_owned());
    let clean = |id: &str| id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_').then(|| id.to_owned()).filter(|id| !id.is_empty());
    let page = |player: String, tall: bool| Some(Video { source: VideoSource::Player(player), tall });
    match host.as_str() {
        "youtube.com" | "music.youtube.com" | "youtube-nocookie.com" => {
            let (id, tall) = match parts.as_slice() {
                ["watch"] => (query("v").as_deref().and_then(clean)?, false),
                ["shorts", id, ..] => (clean(id)?, true),
                ["embed" | "live" | "v", id, ..] => (clean(id)?, false),
                _ => return None,
            };
            page(youtube(&id, query("t").or_else(|| query("start")).as_deref()), tall)
        }
        "youtu.be" => page(youtube(&clean(parts.first()?)?, query("t").as_deref()), false),
        "vimeo.com" => {
            let id = parts.iter().find(|part| part.chars().all(|c| c.is_ascii_digit()))?;
            page(format!("https://player.vimeo.com/video/{id}?autoplay=1"), false)
        }
        "streamable.com" => page(format!("https://streamable.com/e/{}?autoplay=1", clean(parts.first()?)?), false),
        "tiktok.com" => {
            let at = parts.iter().position(|part| *part == "video")?;
            let id = parts.get(at + 1).filter(|id| id.chars().all(|c| c.is_ascii_digit()))?;
            page(format!("https://www.tiktok.com/player/v1/{id}?autoplay=1"), true)
        }
        "dailymotion.com" => match parts.as_slice() {
            ["video", id, ..] => page(format!("https://www.dailymotion.com/embed/video/{}?autoplay=1", clean(id)?), false),
            _ => None,
        },
        "clips.twitch.tv" => page(format!("https://clips.twitch.tv/embed?clip={}&parent={PLAYER_HOST}&autoplay=true", clean(parts.first()?)?), false),
        _ => {
            let path = url.path().to_ascii_lowercase();
            [".mp4", ".webm", ".mov", ".m4v", ".ogv"].iter().any(|ending| path.ends_with(ending)).then(|| Video { source: VideoSource::File(address.to_owned()), tall: false })
        }
    }
}

/// The host the player page is served from, inside the app. Sites that ask who embeds them
/// (YouTube, Twitch) see this name.
pub const PLAYER_HOST: &str = "telinha.localhost";

fn youtube(id: &str, start: Option<&str>) -> String {
    let mut player = format!("https://www.youtube-nocookie.com/embed/{id}?autoplay=1&rel=0&playsinline=1&modestbranding=1");
    if let Some(seconds) = start.and_then(seconds_of) {
        player.push_str(&format!("&start={seconds}"));
    }
    player
}

/// "90", "90s" or "1m30s" as 90.
fn seconds_of(text: &str) -> Option<u32> {
    if let Ok(seconds) = text.parse() {
        return Some(seconds);
    }
    let mut total = 0;
    let mut number = String::new();
    for c in text.chars() {
        match c {
            '0'..='9' => number.push(c),
            'h' | 'm' | 's' => {
                let value: u32 = number.parse().ok()?;
                total += value * match c { 'h' => 3600, 'm' => 60, _ => 1 };
                number.clear();
            }
            _ => return None,
        }
    }
    number.is_empty().then_some(total)
}

/// The page that holds the video: the player of the site in a frame that fills it, or the file
/// in a video element, on black.
pub fn player_page(video: &Video) -> String {
    let escape = |text: &str| text.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;");
    let body = match &video.source {
        VideoSource::Player(player) => format!(r#"<iframe src="{}" allow="autoplay; encrypted-media; picture-in-picture; fullscreen; clipboard-write" allowfullscreen referrerpolicy="strict-origin-when-cross-origin"></iframe>"#, escape(player)),
        VideoSource::File(file) => format!(r#"<video src="{}" controls autoplay playsinline></video>"#, escape(file)),
    };
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="referrer" content="strict-origin-when-cross-origin"><style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}iframe,video{{border:0;width:100%;height:100%;display:block;background:#000}}</style></head><body>{body}</body></html>"#
    )
}


/// The window of the app, to put the player inside it.
#[derive(Clone, Copy)]
pub struct ParentWindow(wry::raw_window_handle::RawWindowHandle);

impl ParentWindow {
    pub fn new(handle: wry::raw_window_handle::RawWindowHandle) -> Self {
        Self(handle)
    }
}

impl wry::raw_window_handle::HasWindowHandle for ParentWindow {
    fn window_handle(&self) -> Result<wry::raw_window_handle::WindowHandle<'_>, wry::raw_window_handle::HandleError> {
        // SAFETY: the handle is of the main window, which lives as long as the app.
        Ok(unsafe { wry::raw_window_handle::WindowHandle::borrow_raw(self.0) })
    }
}

/// The browser that plays videos inside the conversation: one for the whole app, a window of
/// WebView2 laid over the place the conversation keeps for the video.
pub struct Player {
    view: wry::WebView,
    _context: wry::WebContext,
    /// The page the browser asks for next, and a count that makes each address new.
    page: Arc<std::sync::Mutex<String>>,
    loads: u64,
    /// What plays now, and where it is shown, in pixels; None while hidden.
    playing: Option<String>,
    bounds: Option<[i32; 4]>,
}

impl Player {
    /// Opens the browser, hidden. Its files go to `data_dir`, never next to the program.
    pub fn open(parent: &ParentWindow, data_dir: &std::path::Path) -> Result<Self, String> {
        let mut context = wry::WebContext::new(Some(data_dir.to_path_buf()));
        let page = Arc::new(std::sync::Mutex::new(String::new()));
        let served = Arc::clone(&page);
        let view = wry::WebViewBuilder::new_with_web_context(&mut context)
            .with_custom_protocol("telinha".into(), move |_, _| {
                let html = served.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
                wry::http::Response::builder().header("Content-Type", "text/html; charset=utf-8").body(std::borrow::Cow::Owned(html.into_bytes())).unwrap_or_default()
            })
            .with_autoplay(true)
            .with_visible(false)
            .with_background_color((0, 0, 0, 255))
            .with_bounds(wry::Rect { position: wry::dpi::PhysicalPosition::new(0, 0).into(), size: wry::dpi::PhysicalSize::new(16u32, 9u32).into() })
            .build_as_child(parent)
            .map_err(|error| error.to_string())?;
        Ok(Self { view, _context: context, page, loads: 0, playing: None, bounds: None })
    }

    /// Plays `video` under the name `key`, unless it is already what plays.
    pub fn play(&mut self, key: &str, video: &Video) {
        if self.playing.as_deref() == Some(key) {
            return;
        }
        *self.page.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = player_page(video);
        self.loads += 1;
        let _ = self.view.load_url(&format!("http://{PLAYER_HOST}/player/{}", self.loads));
        self.playing = Some(key.to_owned());
    }

    /// Stops the video and hides the browser.
    pub fn stop(&mut self) {
        if self.playing.take().is_some() {
            let _ = self.view.load_url("about:blank");
        }
        self.place(None, 1.0);
    }

    pub fn playing(&self) -> Option<&str> {
        self.playing.as_deref()
    }

    /// Shows the browser over `rect`, in points of the window, or hides it with None. The video
    /// goes on playing while hidden.
    pub fn place(&mut self, rect: Option<egui::Rect>, pixels_per_point: f32) {
        let bounds = rect.map(|rect| {
            let scale = |value: f32| (value * pixels_per_point).round() as i32;
            [scale(rect.left()), scale(rect.top()), scale(rect.width()).max(1), scale(rect.height()).max(1)]
        });
        if bounds == self.bounds {
            return;
        }
        if let Some([x, y, width, height]) = bounds {
            let _ = self.view.set_bounds(wry::Rect { position: wry::dpi::PhysicalPosition::new(x, y).into(), size: wry::dpi::PhysicalSize::new(width as u32, height as u32).into() });
        }
        if bounds.is_some() != self.bounds.is_some() {
            let _ = self.view.set_visible(bounds.is_some());
        }
        self.bounds = bounds;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_links_become_players_of_their_sites() {
        let player = |url: &str| match video_of(url) {
            Some(Video { source: VideoSource::Player(player), tall }) => Some((player, tall)),
            _ => None,
        };
        assert_eq!(player("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1m30s"), Some(("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1&rel=0&playsinline=1&modestbranding=1&start=90".into(), false)));
        assert_eq!(player("https://youtu.be/dQw4w9WgXcQ").map(|(_, tall)| tall), Some(false));
        assert_eq!(player("https://youtube.com/shorts/abcDEF12345").map(|(_, tall)| tall), Some(true));
        assert_eq!(player("https://m.youtube.com/watch?v=abc").map(|(player, _)| player.contains("/embed/abc?")), Some(true));
        assert_eq!(player("https://vimeo.com/76979871").map(|(player, _)| player), Some("https://player.vimeo.com/video/76979871?autoplay=1".into()));
        assert_eq!(player("https://www.tiktok.com/@alguem/video/7311111111111111111").map(|(_, tall)| tall), Some(true));
        assert!(player("https://clips.twitch.tv/CoolClip-abc").is_some_and(|(player, _)| player.contains("parent=telinha.localhost")));
        assert_eq!(video_of("https://example.com/filme.MP4").map(|video| video.source), Some(VideoSource::File("https://example.com/filme.MP4".into())));
        assert_eq!(video_of("https://www.youtube.com/watch?v=a\"b"), None);
        assert_eq!(video_of("https://example.com/receita"), None);
        assert!(player_page(&video_of("https://youtu.be/abc").unwrap()).contains("<iframe src=\"https://www.youtube-nocookie.com/embed/abc?autoplay=1&amp;rel=0"));
    }

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
