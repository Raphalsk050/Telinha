//! A picture opened over the window: it always fits whole, zooms around the cursor, moves by
//! dragging, and what is behind it shows blurred.

use std::sync::{Arc, Mutex};

use egui::{Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, Sense, Vec2};

use crate::icons;
use crate::theme::Palette;

const MARGIN: f32 = 24.0;
/// Room left at the bottom for the zoom bar, so it does not cover the base of a tall picture.
const TOOLS_SPACE: f32 = 72.0;
/// The most a picture grows, in times its real size.
const MAX_SCALE: f32 = 8.0;
const STEP: f32 = 1.25;
const WHEEL: f32 = 0.002;
/// How long to wait for the picture of the window that becomes the blurred backdrop.
const BACKDROP_WAIT: f64 = 0.25;
/// The backdrop is blurred at a fraction of the window's size: cheaper, and it adds to the blur.
const BACKDROP_SHRINK: u32 = 6;

pub struct Lightbox {
    uri: String,
    bytes: Arc<[u8]>,
    /// The size of the picture in its own pixels.
    natural: Vec2,
    /// 1 is the whole picture in the window.
    zoom: f32,
    pan: Vec2,
    backdrop: Option<egui::TextureHandle>,
    /// Where the picture of the window lands when the window system hands it over.
    shot: Arc<Mutex<Option<Arc<egui::ColorImage>>>>,
    opened_at: Option<f64>,
    dragged: bool,
}

/// The part of the window the picture is shown in, and its center.
fn area_of(screen: Rect) -> Rect {
    Rect::from_min_max(screen.min + Vec2::splat(MARGIN), screen.max - Vec2::new(MARGIN, TOOLS_SPACE))
}

/// The scale that shows the whole picture in the area, never above its real size.
pub fn fit_scale(natural: Vec2, area: Vec2) -> f32 {
    (area.x / natural.x).min(area.y / natural.y).min(1.0).max(0.01)
}

/// How far the picture may be moved from the center: until its edge reaches the margin.
pub fn clamp_pan(pan: Vec2, shown: Vec2, area: Vec2) -> Vec2 {
    let reach = ((shown - area) / 2.0).max(Vec2::ZERO);
    Vec2::new(pan.x.clamp(-reach.x, reach.x), pan.y.clamp(-reach.y, reach.y))
}

/// The pan after a zoom that keeps the point under the cursor where it is.
pub fn pan_after_zoom(pan: Vec2, from_center: Vec2, previous: f32, next: f32) -> Vec2 {
    from_center - (from_center - pan) * (next / previous)
}

impl Lightbox {
    /// Opens a picture. Nothing when the bytes are not a picture the app reads.
    pub fn open(context: &egui::Context, uri: String, bytes: Arc<[u8]>) -> Option<Self> {
        let (width, height) = image::ImageReader::new(std::io::Cursor::new(&bytes[..])).with_guessed_format().ok()?.into_dimensions().ok()?;
        // The window as it is now becomes the backdrop, so the picture waits for it to arrive.
        let shot: Arc<Mutex<Option<Arc<egui::ColorImage>>>> = Arc::default();
        let (slot, waker) = (Arc::clone(&shot), context.clone());
        context.request_screenshot(move |image| {
            *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(image);
            waker.request_repaint();
        });
        Some(Self {
            uri,
            bytes,
            natural: Vec2::new(width as f32, height as f32),
            zoom: 1.0,
            pan: Vec2::ZERO,
            backdrop: None,
            shot,
            opened_at: None,
            dragged: false,
        })
    }

    fn take_backdrop(&mut self, context: &egui::Context) {
        let Some(shot) = self.shot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() else { return };
        let [width, height] = shot.size;
        let pixels: Vec<u8> = shot.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
        let Some(picture) = image::RgbaImage::from_raw(width as u32, height as u32, pixels) else { return };
        let small = image::imageops::thumbnail(&picture, (width as u32 / BACKDROP_SHRINK).max(1), (height as u32 / BACKDROP_SHRINK).max(1));
        let blurred = image::imageops::blur(&small, 3.0);
        let size = [blurred.width() as usize, blurred.height() as usize];
        let texture = egui::ColorImage::from_rgba_unmultiplied(size, blurred.as_raw());
        self.backdrop = Some(context.load_texture("lightbox-backdrop", texture, egui::TextureOptions::LINEAR));
    }

    fn zoom_by(&mut self, factor: f32, from_center: Vec2, area: Vec2) {
        let fit = fit_scale(self.natural, area);
        let mut next = (self.zoom * factor).clamp(1.0, MAX_SCALE / fit);
        if next < 1.02 {
            next = 1.0;
        }
        self.pan = pan_after_zoom(self.pan, from_center, self.zoom, next);
        self.zoom = next;
    }

    fn reset(&mut self) {
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
    }

    /// Draws the picture over everything. False when the person closed it.
    pub fn show(&mut self, context: &egui::Context, palette: &Palette) -> bool {
        let now = context.input(|input| input.time);
        let opened_at = *self.opened_at.get_or_insert(now);
        if self.backdrop.is_none() {
            self.take_backdrop(context);
            if self.backdrop.is_none() && now - opened_at < BACKDROP_WAIT {
                context.request_repaint();
                return true;
            }
        }

        let screen = context.content_rect();
        let area = area_of(screen);
        let mut open = true;
        egui::Area::new(egui::Id::unique("lightbox")).order(egui::Order::Foreground).fixed_pos(screen.min).show(context, |ui| {
            let (_, backdrop) = ui.allocate_exact_size(screen.size(), Sense::click());
            let painter = ui.painter().clone();
            match &self.backdrop {
                Some(texture) => {
                    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                    painter.image(texture.id(), screen, uv, Color32::WHITE);
                    painter.rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(64));
                }
                None => {
                    painter.rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(178));
                }
            }

            // The wheel zooms around the cursor, and the keys around the middle.
            let (scroll, pointer) = ui.input(|input| (input.smooth_scroll_delta.y, input.pointer.hover_pos()));
            if scroll != 0.0 {
                let from_center = pointer.map_or(Vec2::ZERO, |pointer| pointer - area.center());
                self.zoom_by((scroll * WHEEL * 4.0).exp(), from_center, area.size());
            }
            ui.input(|input| {
                if input.key_pressed(Key::Plus) || input.key_pressed(Key::Equals) {
                    self.zoom_by(STEP, Vec2::ZERO, area.size());
                }
                if input.key_pressed(Key::Minus) {
                    self.zoom_by(1.0 / STEP, Vec2::ZERO, area.size());
                }
                if input.key_pressed(Key::Num0) {
                    self.reset();
                }
                if input.key_pressed(Key::Escape) {
                    open = false;
                }
            });

            let fit = fit_scale(self.natural, area.size());
            let shown = self.natural * fit * self.zoom;
            self.pan = clamp_pan(self.pan, shown, area.size());
            let rect = Rect::from_center_size(area.center() + self.pan, shown);
            let response = ui.interact(rect, ui.scope_id().with("picture"), Sense::click_and_drag());
            if response.drag_started() {
                self.dragged = false;
            }
            if response.dragged() && response.drag_delta() != Vec2::ZERO {
                self.dragged = true;
                self.pan = clamp_pan(self.pan + response.drag_delta(), shown, area.size());
            }
            // A click goes to the real size of the picture, or to twice it when it is already
            // there, and the next click shows the whole picture again.
            if response.clicked() && !self.dragged {
                if self.zoom > 1.0 {
                    self.reset();
                } else {
                    let from_center = response.interact_pointer_pos().map_or(Vec2::ZERO, |pointer| pointer - area.center());
                    self.zoom_by(if fit < 0.8 { 1.0 / fit } else { 2.0 }, from_center, area.size());
                }
            }
            let cursor = if response.dragged() {
                egui::CursorIcon::Grabbing
            } else if self.zoom > 1.0 {
                egui::CursorIcon::Grab
            } else {
                egui::CursorIcon::ZoomIn
            };
            let response = response.on_hover_cursor(cursor);
            let _ = response;
            egui::Image::from_bytes(self.uri.clone(), egui::load::Bytes::Shared(Arc::clone(&self.bytes)))
                .corner_radius(CornerRadius::same(4))
                .paint_at(ui, rect);

            // The bar of the zoom, at the bottom.
            let bar = Rect::from_center_size(Pos2::new(screen.center().x, screen.bottom() - 16.0 - 21.0), Vec2::new(138.0, 42.0));
            painter.rect_filled(bar, CornerRadius::same(10), Color32::from_black_alpha(153));
            let most = MAX_SCALE / fit;
            let slot = |ui: &mut egui::Ui, index: f32, width: f32, id: &str| {
                let rect = Rect::from_min_size(bar.min + Vec2::new(4.0 + index, 4.0), Vec2::new(width, 34.0));
                let response = ui.interact(rect, ui.scope_id().with(id), Sense::click());
                if response.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), Color32::from_white_alpha(40));
                }
                (rect, response.on_hover_cursor(egui::CursorIcon::PointingHand))
            };
            let (out_rect, out) = slot(ui, 0.0, 34.0, "out");
            let (label_rect, label) = slot(ui, 36.0, 58.0, "fit");
            let (in_rect, zoom_in) = slot(ui, 96.0, 34.0, "in");
            let dim = |enabled: bool| if enabled { Color32::WHITE } else { Color32::from_white_alpha(100) };
            icons::MINUS.image(18.0, dim(self.zoom > 1.0)).paint_at(ui, Rect::from_center_size(out_rect.center(), Vec2::splat(18.0)));
            icons::PLUS.image(18.0, dim(self.zoom < most)).paint_at(ui, Rect::from_center_size(in_rect.center(), Vec2::splat(18.0)));
            let percent = format!("{}%", (fit * self.zoom * 100.0).round());
            painter.text(label_rect.center(), Align2::CENTER_CENTER, percent, FontId::proportional(13.0), Color32::WHITE);
            if out.on_hover_text("Diminuir o zoom").clicked() {
                self.zoom_by(1.0 / STEP, Vec2::ZERO, area.size());
            }
            if zoom_in.on_hover_text("Aumentar o zoom").clicked() {
                self.zoom_by(STEP, Vec2::ZERO, area.size());
            }
            if label.on_hover_text("Ajustar à janela").clicked() {
                self.reset();
            }

            let close = Rect::from_min_size(Pos2::new(screen.right() - 16.0 - 34.0, screen.top() + 16.0), Vec2::splat(34.0));
            let close_response = ui.interact(close, ui.scope_id().with("close"), Sense::click());
            let fill = if close_response.hovered() { Color32::from_black_alpha(217) } else { Color32::from_black_alpha(153) };
            painter.rect_filled(close, CornerRadius::same(6), fill);
            icons::CLOSE.image(18.0, Color32::WHITE).paint_at(ui, Rect::from_center_size(close.center(), Vec2::splat(18.0)));
            let closed = close_response.on_hover_text("Fechar").on_hover_cursor(egui::CursorIcon::PointingHand).clicked();

            // Only the backdrop closes: a click on the picture or on the buttons is for the zoom.
            let on_backdrop = backdrop.clicked() && backdrop.interact_pointer_pos().is_some_and(|pointer| !rect.contains(pointer) && !bar.contains(pointer));
            if closed || on_backdrop {
                open = false;
            }
            let _ = palette;
        });
        open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_fits_whole_and_never_grows_past_its_size() {
        let area = Vec2::new(1848.0, 844.0);
        assert!((fit_scale(Vec2::new(700.0, 1900.0), area) - 844.0 / 1900.0).abs() < 1e-6);
        assert!((fit_scale(Vec2::new(2600.0, 800.0), area) - 1848.0 / 2600.0).abs() < 1e-6);
        assert_eq!(fit_scale(Vec2::new(320.0, 240.0), area), 1.0);
    }

    #[test]
    fn a_picture_moves_only_until_its_edge_reaches_the_margin() {
        let area = Vec2::new(1000.0, 600.0);
        assert_eq!(clamp_pan(Vec2::new(500.0, -500.0), Vec2::new(800.0, 400.0), area), Vec2::ZERO);
        assert_eq!(clamp_pan(Vec2::new(900.0, -900.0), Vec2::new(2000.0, 1000.0), area), Vec2::new(500.0, -200.0));
    }

    #[test]
    fn the_point_under_the_cursor_stays_put_through_a_zoom() {
        let cursor = Vec2::new(200.0, -50.0);
        let pan = pan_after_zoom(Vec2::ZERO, cursor, 1.0, 2.0);
        // The picture point that was under the cursor: (cursor - pan) / zoom, before and after.
        assert_eq!((cursor - Vec2::ZERO) / 1.0, (cursor - pan) / 2.0);
        assert_eq!(pan_after_zoom(pan, cursor, 2.0, 1.0), Vec2::ZERO);
    }
}
