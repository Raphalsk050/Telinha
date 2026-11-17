//! "Selecione uma imagem", as `#avatar-picker`: send a new picture, take one of the last avatars
//! again, or remove the avatar.

use egui::{CornerRadius, FontId, Rect, RichText, Sense, Stroke, Ui, Vec2};
use telinha_core::avatars::Picture;

use crate::dialogs::{Tone, button, dialog_frame, header, section_title};
use crate::theme::{self, Palette};

/// What the person did in the picker.
pub enum PickerAnswer {
    Open,
    Close,
    /// A picture to crop into the avatar.
    Picked(Vec<u8>),
    Recent(String),
    Remove,
}

pub struct AvatarPicker {
    recents: Vec<Picture>,
    has_avatar: bool,
    status: String,
}

/// As `.avatar-dialog`: 420 wide with the padding of a dialog.
const INNER: f32 = 420.0 - 40.0;

impl AvatarPicker {
    pub fn new(recents: Vec<Picture>, has_avatar: bool) -> Self {
        Self { recents, has_avatar, status: String::new() }
    }

    pub fn show(&mut self, context: &egui::Context, palette: &Palette) -> PickerAnswer {
        let mut answer = PickerAnswer::Open;
        // A picture dragged onto the window counts as the picture sent.
        let dropped = context.input(|input| input.raw.dropped_files.first().cloned());
        if let Some(file) = dropped {
            match file.bytes() {
                Ok(bytes) => return PickerAnswer::Picked(bytes),
                Err(_) => self.status = "Não consegui ler essa imagem.".to_owned(),
            }
        }
        let dragging = context.input(|input| !input.raw.hovered_files.is_empty());
        let response = crate::glass::modal(context, egui::Id::unique("avatar-picker"), dialog_frame(palette), |ui| {
                ui.set_width(INNER);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                if header(ui, palette, "Selecione uma imagem") {
                    answer = PickerAnswer::Close;
                }
                if upload_area(ui, palette, dragging) {
                    match crate::settings::pick_picture_file() {
                        Some(Ok(bytes)) => answer = PickerAnswer::Picked(bytes),
                        Some(Err(text)) => self.status = text,
                        None => {}
                    }
                }
                if !self.status.is_empty() {
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(RichText::new(&self.status).color(palette.danger_text).size(13.0)).wrap());
                }
                if !self.recents.is_empty() {
                    section_title(ui, palette, "AVATARES RECENTES");
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(10.0);
                        for recent in &self.recents {
                            if recent_button(ui, palette, recent) {
                                answer = PickerAnswer::Recent(recent.hash.clone());
                            }
                        }
                    });
                }
                if self.has_avatar {
                    ui.add_space(20.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if button(ui, palette, "Remover avatar", Tone::Ghost).clicked() {
                            answer = PickerAnswer::Remove;
                        }
                    });
                }
            });
        if response.should_close() && matches!(answer, PickerAnswer::Open) {
            answer = PickerAnswer::Close;
        }
        answer
    }
}

/// The big dashed box that opens the file picker, as `.avatar-upload`.
fn upload_area(ui: &mut Ui, palette: &Palette, dragging: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(INNER, 150.0), Sense::click());
    let lit = response.hovered() || dragging;
    ui.painter().rect_filled(rect, CornerRadius::same(8), if lit { palette.hover } else { palette.sidebar });
    let border = Stroke::new(1.0, if lit { palette.accent } else { palette.line_soft });
    let edge = rect.shrink(0.5);
    let corners = [edge.left_top(), edge.right_top(), edge.right_bottom(), edge.left_bottom(), edge.left_top()];
    ui.painter().extend(egui::Shape::dashed_line(&corners, border, 4.0, 3.0));
    let icon = Rect::from_center_size(egui::pos2(rect.center().x, rect.center().y - 26.0), Vec2::splat(36.0));
    crate::icons::UPLOAD_PICTURE.image(36.0, palette.strong).paint_at(ui, icon);
    ui.painter().text(egui::pos2(rect.center().x, rect.center().y + 10.0), egui::Align2::CENTER_CENTER, "Enviar imagem", FontId::new(15.0, theme::semibold()), palette.strong);
    ui.painter().text(egui::pos2(rect.center().x, rect.center().y + 32.0), egui::Align2::CENTER_CENTER, "ou arraste uma imagem para cá", FontId::proportional(13.0), palette.muted);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// One of the last avatars, round, as `.avatar-recent`.
fn recent_button(ui: &mut Ui, palette: &Palette, recent: &Picture) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(52.0), Sense::click());
    ui.painter().circle_filled(rect.center(), 26.0, palette.rail);
    let uri = format!("bytes://recent-avatar/{}", recent.hash);
    egui::Image::from_bytes(uri, recent.data.clone()).corner_radius(CornerRadius::same(26)).paint_at(ui, rect.shrink(2.0));
    if response.hovered() {
        ui.painter().circle_stroke(rect.center(), 25.0, Stroke::new(2.0, palette.accent));
    }
    response.on_hover_text("Usar este avatar").on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}
