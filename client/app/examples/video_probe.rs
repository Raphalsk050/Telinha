//! Opens a window with the video player of the chat over part of it and plays the link given,
//! to see that the sites play inside the app: `cargo run --example video_probe -- <link>`.

use telinha::media::{ParentWindow, Player, video_of};

struct Probe {
    player: Option<Player>,
    link: String,
    data: std::path::PathBuf,
    parent: Option<ParentWindow>,
}

impl eframe::App for Probe {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.label(format!("Tocando {}", self.link));
        if self.player.is_none() {
            if let Some(parent) = self.parent {
                match Player::open(&parent, &self.data) {
                    Ok(player) => self.player = Some(player),
                    Err(error) => eprintln!("player: {error}"),
                }
            }
        }
        if let (Some(player), Some(video)) = (&mut self.player, video_of(&self.link)) {
            player.play("probe", &video);
            player.place(Some(egui::Rect::from_min_size(egui::pos2(20.0, 40.0), egui::vec2(640.0, 360.0))), ui.ctx().pixels_per_point());
        }
    }
}

fn main() -> eframe::Result {
    let link = std::env::args().nth(1).unwrap_or_else(|| "https://www.youtube.com/watch?v=aqz-KE-bpKQ".to_owned());
    let data = std::env::temp_dir().join("telinha-video-probe");
    let viewport = egui::ViewportBuilder::default().with_inner_size([680.0, 420.0]).with_position([100.0, 100.0]).with_title("video probe");
    eframe::run_native(
        "video probe",
        eframe::NativeOptions { viewport, ..Default::default() },
        Box::new(move |creation| {
            let parent = wry::raw_window_handle::HasWindowHandle::window_handle(creation).ok().map(|window| ParentWindow::new(window.as_raw()));
            Ok(Box::new(Probe { player: None, link, data, parent }))
        }),
    )
}
