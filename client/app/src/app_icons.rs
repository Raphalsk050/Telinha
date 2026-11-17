//! The icons the person can pick for the window and the taskbar, as Discord lets pick the icon
//! of its app. The file of the program keeps its own icon; this one replaces it while the app is
//! open.

use std::sync::Arc;

/// The id kept in the preferences, the name shown and the picture, 256 pixels square.
pub const ICONS: [(&str, &str, &[u8]); 8] = [
    ("original", "Original", include_bytes!("../assets/app_icons/original.png")),
    ("roxo", "Roxo", include_bytes!("../assets/app_icons/roxo.png")),
    ("oceano", "Oceano", include_bytes!("../assets/app_icons/oceano.png")),
    ("por-do-sol", "Pôr do sol", include_bytes!("../assets/app_icons/por-do-sol.png")),
    ("menta", "Menta", include_bytes!("../assets/app_icons/menta.png")),
    ("blurple", "Anil", include_bytes!("../assets/app_icons/blurple.png")),
    ("grafite", "Grafite", include_bytes!("../assets/app_icons/grafite.png")),
    ("vidro", "Vidro", include_bytes!("../assets/app_icons/vidro.png")),
];

pub const DEFAULT: &str = "original";

/// The picture of `id`, or of the original when the id is unknown.
pub fn bytes(id: &str) -> &'static [u8] {
    ICONS.iter().find(|(known, _, _)| *known == id).unwrap_or(&ICONS[0]).2
}

/// A known id, or the default.
pub fn known(id: &str) -> String {
    if ICONS.iter().any(|(known, _, _)| *known == id) { id.to_owned() } else { DEFAULT.to_owned() }
}

/// Puts the icon of `id` on the window and its button in the taskbar.
pub fn apply(context: &egui::Context, id: &str) {
    let Ok(picture) = image::load_from_memory(bytes(id)) else { return };
    let picture = picture.to_rgba8();
    let (width, height) = picture.dimensions();
    context.send_viewport_cmd(egui::ViewportCommand::Icon(Some(Arc::new(egui::IconData { rgba: picture.into_raw(), width, height }))));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_square_picture_and_unknown_ids_fall_back() {
        for (id, _, bytes) in ICONS {
            let picture = image::load_from_memory(bytes).unwrap_or_else(|_| panic!("{id} opens"));
            assert_eq!((picture.width(), picture.height()), (256, 256), "{id}");
        }
        assert_eq!(known("nada"), DEFAULT);
        assert_eq!(known("menta"), "menta");
    }
}
