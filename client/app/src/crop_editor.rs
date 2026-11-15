//! Picks which part of a picture becomes the avatar or the banner: the person drags and zooms
//! the picture behind a frame, and what is inside the frame is what gets saved.

use egui::{Align, Color32, CornerRadius, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use image::{DynamicImage, GenericImageView};

use crate::dialogs::{Tone, button};
use crate::settings::{chip_button, encode_picture};
use crate::theme::{self, Palette};

const MAX_ZOOM: f32 = 4.0;
/// The longest side of the copy of the picture that is shown while editing.
const SHOWN_SIDE: u32 = 1400;

/// The frame of each kind of picture: the room around it, the frame itself, and the size of
/// what comes out.
struct Shape {
    view: Vec2,
    crop: Vec2,
    output: (u32, u32),
    round: bool,
    title: &'static str,
}

const AVATAR: Shape = Shape { view: Vec2::new(300.0, 300.0), crop: Vec2::new(240.0, 240.0), output: (256, 256), round: true, title: "Editar imagem" };
const BANNER: Shape = Shape { view: Vec2::new(480.0, 220.0), crop: Vec2::new(440.0, 154.0), output: (600, 210), round: false, title: "Recortar banner" };

/// How many times the picture is enlarged on screen, so that at zoom 1 it just fills the frame.
fn scale_of(picture: Vec2, crop: Vec2, zoom: f32) -> f32 {
    (crop.x / picture.x).max(crop.y / picture.y) * zoom
}

/// Keeps the picture from leaving a piece of the frame empty.
pub fn clamp_pan(picture: Vec2, crop: Vec2, zoom: f32, pan: Vec2) -> Vec2 {
    let shown = picture * scale_of(picture, crop, zoom);
    let most = ((shown - crop) / 2.0).max(Vec2::ZERO);
    Vec2::new(pan.x.clamp(-most.x, most.x), pan.y.clamp(-most.y, most.y))
}

/// The part of the picture that is inside the frame, in pixels of the picture: left, top, width
/// and height.
pub fn source_rect(picture: (u32, u32), crop: Vec2, zoom: f32, pan: Vec2) -> (u32, u32, u32, u32) {
    let size = Vec2::new(picture.0 as f32, picture.1 as f32);
    let scale = scale_of(size, crop, zoom);
    let pan = clamp_pan(size, crop, zoom, pan);
    // The picture is centered on the frame and then moved by the pan.
    let left = (size.x * scale - crop.x) / 2.0 - pan.x;
    let top = (size.y * scale - crop.y) / 2.0 - pan.y;
    let width = ((crop.x / scale).round() as u32).clamp(1, picture.0);
    let height = ((crop.y / scale).round() as u32).clamp(1, picture.1);
    let x = ((left / scale).round().max(0.0) as u32).min(picture.0 - width);
    let y = ((top / scale).round().max(0.0) as u32).min(picture.1 - height);
    (x, y, width, height)
}

pub struct CropEditor {
    banner: bool,
    picture: DynamicImage,
    texture: Option<egui::TextureHandle>,
    zoom: f32,
    pan: Vec2,
}

/// What the person decided in the editor.
pub enum CropAnswer {
    Open,
    Cancel,
    Save { banner: bool, data: Vec<u8> },
    Failed(String),
}

impl CropEditor {
    pub fn open(bytes: &[u8], banner: bool) -> Result<Self, String> {
        let picture = image::load_from_memory(bytes).map_err(|_| "Não consegui abrir essa imagem. Tente PNG, JPG, WebP ou GIF.")?;
        if picture.width() == 0 || picture.height() == 0 {
            return Err("Essa imagem está vazia.".into());
        }
        Ok(Self { banner, picture, texture: None, zoom: 1.0, pan: Vec2::ZERO })
    }

    fn shape(&self) -> &'static Shape {
        if self.banner { &BANNER } else { &AVATAR }
    }

    fn size(&self) -> Vec2 {
        Vec2::new(self.picture.width() as f32, self.picture.height() as f32)
    }

    fn texture(&mut self, context: &egui::Context) -> egui::TextureHandle {
        if let Some(texture) = &self.texture {
            return texture.clone();
        }
        let shown = self.picture.thumbnail(SHOWN_SIDE, SHOWN_SIDE).to_rgba8();
        let pixels = egui::ColorImage::from_rgba_unmultiplied([shown.width() as usize, shown.height() as usize], shown.as_raw());
        let texture = context.load_texture("crop-editor", pixels, egui::TextureOptions::LINEAR);
        self.texture = Some(texture.clone());
        texture
    }

    fn set_zoom(&mut self, zoom: f32, around: Vec2) {
        let zoom = zoom.clamp(1.0, MAX_ZOOM);
        // The point under the cursor stays where it is while the picture grows around it.
        let factor = zoom / self.zoom;
        self.pan = around + (self.pan - around) * factor;
        self.zoom = zoom;
        self.pan = clamp_pan(self.size(), self.shape().crop, self.zoom, self.pan);
    }

    /// The part inside the frame, at the size and the weight a profile picture has.
    pub fn export(&self) -> Result<Vec<u8>, String> {
        let shape = self.shape();
        let (x, y, width, height) = source_rect(self.picture.dimensions(), shape.crop, self.zoom, self.pan);
        encode_picture(&self.picture.crop_imm(x, y, width, height), shape.output)
    }

    pub fn show(&mut self, context: &egui::Context, palette: &Palette) -> CropAnswer {
        let shape = self.shape();
        let texture = self.texture(context);
        let mut answer = CropAnswer::Open;
        let frame = Frame::new().fill(palette.main).corner_radius(CornerRadius::same(8)).stroke(Stroke::new(1.0, palette.line)).inner_margin(Margin::same(20));
        let response = egui::Modal::new(egui::Id::unique("crop-editor")).frame(frame).backdrop_color(Color32::from_black_alpha(178)).show(context, |ui| {
            ui.set_width(shape.view.x.max(360.0));
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.label(RichText::new(shape.title).color(palette.strong).size(20.0).family(theme::bold()));
            ui.label(RichText::new("Arraste para posicionar e use a roda do mouse para aproximar.").color(palette.muted).size(13.0));

            ui.vertical_centered(|ui| {
                let (view, response) = ui.allocate_exact_size(shape.view, Sense::drag());
                if response.dragged() {
                    self.pan = clamp_pan(self.size(), shape.crop, self.zoom, self.pan + response.drag_delta());
                }
                if let Some(pointer) = response.hover_pos() {
                    let wheel = ui.input(|input| input.smooth_scroll_delta.y);
                    if wheel != 0.0 {
                        self.set_zoom(self.zoom * (wheel * 0.0025).exp(), pointer - view.center());
                    }
                }
                let painter = ui.painter().with_clip_rect(view);
                painter.rect_filled(view, CornerRadius::same(6), Color32::BLACK);
                let shown = self.size() * scale_of(self.size(), shape.crop, self.zoom);
                let place = Rect::from_center_size(view.center() + self.pan, shown);
                painter.image(texture.id(), place, Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);

                // What is outside the frame is dimmed, so the frame reads as a window.
                let dim = Color32::from_black_alpha(150);
                let hole = Rect::from_center_size(view.center(), shape.crop);
                if shape.round {
                    // A ring from the circle out past the corners of the view, built by hand: a
                    // stroke this thick does not come out as a ring.
                    let radius = shape.crop.x / 2.0;
                    let far = view.size().length() / 2.0 + 2.0;
                    let mut ring = egui::Mesh::default();
                    const STEPS: u32 = 72;
                    for step in 0..=STEPS {
                        let direction = Vec2::angled(std::f32::consts::TAU * step as f32 / STEPS as f32);
                        ring.colored_vertex(view.center() + direction * radius, dim);
                        ring.colored_vertex(view.center() + direction * far, dim);
                    }
                    for step in 0..STEPS {
                        let at = step * 2;
                        ring.add_triangle(at, at + 1, at + 2);
                        ring.add_triangle(at + 2, at + 1, at + 3);
                    }
                    painter.add(egui::Shape::mesh(ring));
                    painter.circle_stroke(view.center(), radius, Stroke::new(1.5, Color32::from_white_alpha(210)));
                } else {
                    for band in [
                        Rect::from_min_max(view.min, egui::pos2(view.right(), hole.top())),
                        Rect::from_min_max(egui::pos2(view.left(), hole.bottom()), view.max),
                        Rect::from_min_max(egui::pos2(view.left(), hole.top()), egui::pos2(hole.left(), hole.bottom())),
                        Rect::from_min_max(egui::pos2(hole.right(), hole.top()), egui::pos2(view.right(), hole.bottom())),
                    ] {
                        painter.rect_filled(band, CornerRadius::ZERO, dim);
                    }
                    painter.rect_stroke(hole, CornerRadius::same(8), Stroke::new(1.5, Color32::from_white_alpha(210)), egui::StrokeKind::Inside);
                }
                response.on_hover_cursor(egui::CursorIcon::Grab);
            });

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if chip_button(ui, palette, "−", false).on_hover_text("Afastar").clicked() {
                    self.set_zoom(self.zoom / 1.25, Vec2::ZERO);
                }
                if chip_button(ui, palette, "+", false).on_hover_text("Aproximar").clicked() {
                    self.set_zoom(self.zoom * 1.25, Vec2::ZERO);
                }
                ui.label(RichText::new(format!("{:.0}%", self.zoom * 100.0)).color(palette.muted).size(13.0));
                if chip_button(ui, palette, "Girar", false).clicked() {
                    self.picture = self.picture.rotate90();
                    self.texture = None;
                    self.pan = Vec2::ZERO;
                    self.zoom = 1.0;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, palette, "Salvar", Tone::Primary).clicked() {
                        answer = match self.export() {
                            Ok(data) => CropAnswer::Save { banner: self.banner, data },
                            Err(error) => CropAnswer::Failed(error),
                        };
                    }
                    if button(ui, palette, "Cancelar", Tone::Plain).clicked() {
                        answer = CropAnswer::Cancel;
                    }
                });
            });
        });
        if response.should_close() && matches!(answer, CropAnswer::Open) {
            answer = CropAnswer::Cancel;
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_takes_the_middle_at_first_and_follows_the_drag_and_the_zoom() {
        let crop = Vec2::new(240.0, 240.0);
        // A wide picture fills the frame by its height, so the middle square is what is inside.
        assert_eq!(source_rect((900, 300), crop, 1.0, Vec2::ZERO), (300, 0, 300, 300));
        // Dragging the picture all the way to the right shows its left end.
        assert_eq!(source_rect((900, 300), crop, 1.0, Vec2::new(9999.0, 0.0)), (0, 0, 300, 300));
        assert_eq!(source_rect((900, 300), crop, 1.0, Vec2::new(-9999.0, 50.0)), (600, 0, 300, 300));
        // Twice the zoom shows half of each side, still around the middle.
        assert_eq!(source_rect((900, 300), crop, 2.0, Vec2::ZERO), (375, 75, 150, 150));
        // A banner out of a square picture is a strip across its middle.
        assert_eq!(source_rect((1000, 1000), Vec2::new(440.0, 154.0), 1.0, Vec2::ZERO), (0, 325, 1000, 350));
    }

    #[test]
    fn the_pan_never_uncovers_the_frame() {
        let crop = Vec2::new(240.0, 240.0);
        assert_eq!(clamp_pan(Vec2::new(900.0, 300.0), crop, 1.0, Vec2::new(500.0, 500.0)), Vec2::new(240.0, 0.0));
        assert_eq!(clamp_pan(Vec2::new(300.0, 300.0), crop, 1.0, Vec2::new(-5.0, 7.0)), Vec2::ZERO);
        assert_eq!(clamp_pan(Vec2::new(300.0, 300.0), crop, 2.0, Vec2::new(-500.0, 7.0)), Vec2::new(-120.0, 7.0));
    }

    #[test]
    fn what_is_saved_is_the_part_inside_the_frame_at_the_size_of_a_profile_picture() {
        let wide = DynamicImage::ImageRgb8(image::RgbImage::from_fn(900, 300, |x, _| if x < 300 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) }));
        let mut bytes = Vec::new();
        wide.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        let mut editor = CropEditor::open(&bytes, false).unwrap();
        let middle = image::load_from_memory(&editor.export().unwrap()).unwrap();
        assert_eq!(middle.dimensions(), (256, 256));
        assert!(middle.get_pixel(20, 128)[2] > 200, "the middle of the picture is blue");
        editor.pan = Vec2::new(9999.0, 0.0);
        let left = image::load_from_memory(&editor.export().unwrap()).unwrap();
        assert!(left.get_pixel(128, 128)[0] > 200, "dragged to the right, the red left end is inside");

        let mut banner = CropEditor::open(&bytes, true).unwrap();
        banner.set_zoom(2.0, Vec2::ZERO);
        assert_eq!(image::load_from_memory(&banner.export().unwrap()).unwrap().dimensions(), (600, 210));
        assert!(CropEditor::open(b"not a picture", false).is_err());
    }
}
