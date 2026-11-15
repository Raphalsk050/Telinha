//! Small pieces drawn by hand so they look the same everywhere: avatars, rows, icon buttons.

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Response, Sense, Ui, Vec2};

use crate::icons::Icon;
use crate::theme::{self, Palette};

/// A round picture, or the initial of the name on a color that is always the same for that name.
pub fn avatar(ui: &mut Ui, side: f32, name: &str, picture: Option<egui::Image<'_>>) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    paint_avatar(ui, rect, name, picture);
    response
}

pub fn paint_avatar(ui: &Ui, rect: Rect, name: &str, picture: Option<egui::Image<'_>>) {
    paint_avatar_with(ui, rect, name, picture, rect.width() * 0.42);
}

/// An avatar whose initial has a size of its own, as the large ones of the call stage have.
pub fn paint_avatar_with(ui: &Ui, rect: Rect, name: &str, picture: Option<egui::Image<'_>>, letter: f32) {
    let radius = rect.width() / 2.0;
    match picture {
        Some(picture) => {
            picture.corner_radius(CornerRadius::same(radius.round() as u8)).paint_at(ui, rect);
        }
        None => {
            ui.painter().circle_filled(rect.center(), radius, theme::avatar_color(name));
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                theme::initial(name),
                FontId::new(letter, theme::semibold()),
                Color32::WHITE,
            );
        }
    }
}

/// The small circle on the corner of an avatar that says whether the person is online.
pub fn status_dot(ui: &Ui, avatar: Rect, online: bool, palette: &Palette, backdrop: Color32) {
    // As the old app drew it: 14 pixels across counting a ring of 3, hanging 2 pixels past the
    // corner of the avatar. The large avatar of a profile card gets a larger one.
    let (size, ring) = if avatar.width() > 56.0 { (22.0, 4.0) } else { (14.0, 3.0) };
    let center = avatar.right_bottom() + Vec2::splat(2.0 - size / 2.0);
    ui.painter().circle_filled(center, size / 2.0, backdrop);
    let fill = if online { palette.green } else { Color32::from_rgb(0x80, 0x84, 0x8e) };
    ui.painter().circle_filled(center, size / 2.0 - ring, fill);
}

/// A square button that is only an icon, with a rounded background on hover.
pub fn icon_button(ui: &mut Ui, icon: Icon, side: f32, color: Color32, hover: Color32, tip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), hover);
    }
    let inner = Rect::from_center_size(rect.center(), Vec2::splat(side * 0.62));
    icon.image(inner.width(), color).paint_at(ui, inner);
    response.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A full-width row that lights up on hover and stays lit when selected. The contents are drawn
/// by the caller inside the returned rectangle.
pub fn row(ui: &mut Ui, height: f32, selected: bool, palette: &Palette) -> (Rect, Response) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let fill = if selected {
        Some(palette.selected)
    } else if response.hovered() {
        Some(palette.hover)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
    }
    (rect, response.on_hover_cursor(egui::CursorIcon::PointingHand))
}

/// Text cut with an ellipsis when it does not fit in `width`, painted at `left_center`.
pub fn paint_text_clipped(ui: &Ui, left_center: egui::Pos2, width: f32, text: &str, font: FontId, color: Color32) -> f32 {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: width, max_rows: 1, break_anywhere: true, ..Default::default() };
    let galley = ui.painter().layout_job(job);
    let used = galley.size().x;
    ui.painter().galley(left_center - Vec2::new(0.0, galley.size().y / 2.0), galley, color);
    used
}
