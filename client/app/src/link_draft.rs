//! The preview of the first link in the message being written. It is fetched while the person
//! types and goes along with the message, so who receives it never has to open the page.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Align, CornerRadius, Frame, Layout, Margin, Rect, RichText, Sense, Ui, Vec2};
use serde_json::{Value, json};
use telinha_core::linkpreview::{LinkPreviews, Preview};
use telinha_core::text::encode_base64;

use crate::attach;
use crate::chat_view::first_link;
use crate::icons;
use crate::theme::{self, Palette};
use crate::widgets;

/// How long the typing has to rest before the link is looked up.
const DELAY: f64 = 0.4;
/// How long a message waits for a preview that is still loading.
pub const SEND_WAIT: f64 = 2.5;
/// The room the card takes above the box.
const CARD_HEIGHT: f32 = 76.0;

/// A preview ready to go with a message.
#[derive(Clone)]
pub struct Found {
    /// The preview as the message carries it.
    pub embed: Value,
    site: String,
    title: String,
    description: String,
    thumb: Option<(String, Arc<[u8]>)>,
}

enum State {
    Loading,
    Ready(Found),
    Nothing,
}

/// What the message being sent should do about its link.
pub enum ForSend {
    /// No link, or a link without a preview.
    Plain,
    Wait,
    Embed(Value),
}

pub struct LinkDraft {
    previews: Arc<LinkPreviews>,
    current: Option<(String, State)>,
    due: Option<f64>,
    last_text: String,
    /// Links whose preview the person closed. They stay closed while the app is open.
    dismissed: HashSet<String>,
    sender: Sender<(String, Option<Found>)>,
    receiver: Receiver<(String, Option<Found>)>,
}

impl Default for LinkDraft {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self { previews: Arc::new(LinkPreviews::default()), current: None, due: None, last_text: String::new(), dismissed: HashSet::new(), sender, receiver }
    }
}

fn host_of(address: &str) -> String {
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    host.strip_prefix("www.").unwrap_or(host).to_owned()
}

/// Shrinks the picture of the page and writes the preview the way a message carries it.
fn build(address: &str, preview: Preview) -> Option<Found> {
    let image = preview.image.as_ref().and_then(|image| image::load_from_memory(&image.data).ok()).and_then(|picture| attach::encode_embed_image(&picture));
    if preview.title.is_empty() && preview.description.is_empty() && image.is_none() {
        return None;
    }
    let embed = json!({
        "url": address,
        "title": preview.title,
        "description": preview.description,
        "siteName": preview.site_name,
        "image": image.as_ref().map(|image| json!({ "mime": image.mime, "width": image.width, "height": image.height, "data": encode_base64(&image.data) })),
    });
    let thumb = image.map(|image| (format!("bytes://link-draft/{}.webp", telinha_core::transfers::sha256_hex(&image.data)), Arc::from(image.data)));
    Some(Found { embed, site: preview.site_name, title: preview.title, description: preview.description, thumb })
}

impl LinkDraft {
    pub fn clear(&mut self) {
        self.current = None;
        self.due = None;
        self.last_text.clear();
    }

    /// Looks at the first link of the draft and starts fetching its preview when it changed.
    fn refresh(&mut self, context: &egui::Context) {
        self.due = None;
        let Some(url) = first_link(&self.last_text).filter(|url| !self.dismissed.contains(url)) else {
            self.current = None;
            return;
        };
        if self.current.as_ref().is_some_and(|(current, _)| *current == url) {
            return;
        }
        self.current = Some((url.clone(), State::Loading));
        let (previews, sender, wake) = (Arc::clone(&self.previews), self.sender.clone(), context.clone());
        // The window draws inside the async runtime, which is where the fetch runs.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        runtime.spawn(async move {
            let preview = previews.get(&url).await;
            let address = url.clone();
            let found = match preview {
                Some(preview) => tokio::task::spawn_blocking(move || build(&address, preview)).await.ok().flatten(),
                None => None,
            };
            let _ = sender.send((url, found));
            wake.request_repaint();
        });
    }

    /// Runs on every draw of the box, with what is written in it.
    pub fn tend(&mut self, context: &egui::Context, draft: &str) {
        let now = context.input(|input| input.time);
        if draft != self.last_text {
            self.last_text = draft.to_owned();
            self.due = Some(now + DELAY);
            context.request_repaint_after(std::time::Duration::from_secs_f64(DELAY));
        }
        if self.due.is_some_and(|due| now >= due) {
            self.refresh(context);
        }
        while let Ok((url, found)) = self.receiver.try_recv() {
            if let Some((current, state)) = &mut self.current {
                if *current == url {
                    *state = found.map_or(State::Nothing, State::Ready);
                }
            }
        }
    }

    /// The preview for a message with this text, about to be sent.
    pub fn for_send(&mut self, context: &egui::Context, text: &str) -> ForSend {
        if self.due.is_some() || text != self.last_text {
            self.last_text = text.to_owned();
            self.refresh(context);
        }
        match &self.current {
            Some((url, state)) if first_link(text).as_ref() == Some(url) => match state {
                State::Loading => ForSend::Wait,
                State::Ready(found) => ForSend::Embed(found.embed.clone()),
                State::Nothing => ForSend::Plain,
            },
            _ => ForSend::Plain,
        }
    }

    pub fn loading(&self) -> bool {
        matches!(self.current, Some((_, State::Loading)))
    }

    /// The room the card needs above the box, when it shows.
    pub fn height(&self) -> f32 {
        match &self.current {
            Some((_, State::Loading | State::Ready(_))) => CARD_HEIGHT + 4.0,
            _ => 0.0,
        }
    }

    /// Shows a made-up preview. For the tests.
    pub fn show_for_test(&mut self, url: &str, site: &str, title: &str, description: &str) {
        let found = Found { embed: Value::Null, site: site.to_owned(), title: title.to_owned(), description: description.to_owned(), thumb: None };
        self.current = Some((url.to_owned(), State::Ready(found)));
    }

    /// The card above the box, with a way to send the message without it.
    pub fn show(&mut self, ui: &mut Ui, palette: &Palette) {
        let Some((url, state)) = &self.current else { return };
        let (site, title, description, thumb) = match state {
            State::Loading => (host_of(url), "Carregando a prévia do link…".to_owned(), String::new(), None),
            State::Ready(found) => (
                if found.site.is_empty() { host_of(url) } else { found.site.clone() },
                if found.title.is_empty() { url.clone() } else { found.title.clone() },
                found.description.clone(),
                found.thumb.clone(),
            ),
            State::Nothing => return,
        };
        let mut close = false;
        Frame::new().fill(palette.panel).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(CARD_HEIGHT - 16.0);
            ui.horizontal(|ui| {
                if let Some((uri, bytes)) = thumb {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(96.0, CARD_HEIGHT - 16.0), Sense::hover());
                    egui::Image::from_bytes(uri, egui::load::Bytes::Shared(bytes)).corner_radius(CornerRadius::same(4)).paint_at(ui, Rect::from_center_size(rect.center(), rect.size()));
                }
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    close = widgets::icon_button(ui, icons::CLOSE, 28.0, palette.soft, palette.selected, "Enviar sem a prévia").clicked();
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(egui::Label::new(RichText::new(site).color(palette.muted).size(12.0)).truncate());
                        ui.add(egui::Label::new(RichText::new(title).color(palette.link).size(14.0).family(theme::semibold())).truncate());
                        if !description.is_empty() {
                            ui.add(egui::Label::new(RichText::new(description).color(palette.text).size(13.0)).truncate());
                        }
                    });
                });
            });
        });
        ui.add_space(4.0);
        if close {
            self.dismissed.insert(url.clone());
            self.current = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_is_written_the_way_a_message_carries_it() {
        let picture = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(1200, 630, image::Rgb([200, 80, 40])));
        let mut data = Vec::new();
        picture.write_to(&mut std::io::Cursor::new(&mut data), image::ImageFormat::Png).unwrap();
        let preview = Preview {
            url: "https://example.com/receita".into(),
            site_name: "example.com".into(),
            title: "Bolo".into(),
            description: "Fofinho".into(),
            image: Some(telinha_core::linkpreview::PreviewImage { data, mime: "image/png".into() }),
        };
        let found = build("https://example.com/receita", preview.clone()).unwrap();
        assert_eq!(found.embed["url"], "https://example.com/receita");
        assert_eq!((found.embed["title"].as_str(), found.embed["siteName"].as_str()), (Some("Bolo"), Some("example.com")));
        assert_eq!((found.embed["image"]["width"].as_i64(), found.embed["image"]["height"].as_i64()), (Some(640), Some(336)));
        assert_eq!(found.embed["image"]["mime"], "image/webp");
        // What the hub takes as a preview is exactly this.
        let decoded = telinha_core::chats::decode_embed(Some(&found.embed)).unwrap();
        assert_eq!((decoded.title.as_str(), decoded.image.is_some()), ("Bolo", true));

        let empty = Preview { title: String::new(), description: String::new(), image: None, ..preview };
        assert!(build("https://example.com/receita", empty).is_none());
        assert_eq!(host_of("https://www.example.com/a/b?c"), "example.com");
    }
}
