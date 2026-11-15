//! The quality and the sound of a share: picked with what to share, and changed while it runs
//! without stopping it, as the blocks `buildQuality` and `buildAudio` of share.js drew them.

use egui::{Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, Ui, Vec2};
use serde_json::Value;
use telinha_core::hub::{ManualRole, audio_command, quality_command};

use crate::app::TelinhaApp;
use crate::dialogs::{dialog_frame, header, section_title};
use crate::theme::{self, Palette};
use crate::widgets;

const RESOLUTIONS: [(i64, i64, &str); 5] = [(0, 0, "Original"), (2560, 1440, "1440p"), (1920, 1080, "1080p"), (1280, 720, "720p"), (854, 480, "480p")];
const FRAMERATES: [(i64, &str); 6] = [(0, "Automático (até 60)"), (120, "120"), (60, "60"), (30, "30"), (15, "15"), (5, "5")];
const BITRATES: [(i64, &str); 6] = [(0, "Automática"), (20_000, "20 Mbps"), (10_000, "10 Mbps"), (6_000, "6 Mbps"), (3_000, "3 Mbps"), (1_500, "1,5 Mbps")];
/// The width of the window of settings of a running share, as `.dialog`, less its padding.
const SETTINGS_WIDTH: f32 = 420.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    #[default]
    Auto,
    Games,
    Screen,
    Custom,
}

const PRESETS: [(Preset, &str, &str); 4] = [
    (Preset::Auto, "Automática", "o Telinha ajusta sozinho"),
    (Preset::Games, "Jogos", "1440p · 60 fps"),
    (Preset::Screen, "Compartilhamento de tela", "texto nítido · 15 fps"),
    (Preset::Custom, "Personalizada", "você escolhe"),
];

/// The limits the person put on the picture. The three numbers of a custom choice are places in
/// the lists above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Quality {
    pub preset: Preset,
    pub resolution: usize,
    pub fps: usize,
    pub bitrate: usize,
}

impl Quality {
    /// The most frames per second, the largest width and height and the most kilobits per
    /// second. Zero leaves it to the engine.
    pub fn limits(&self) -> (i64, i64, i64, i64) {
        match self.preset {
            Preset::Auto => (0, 0, 0, 0),
            Preset::Games => (60, 2560, 1440, 0),
            Preset::Screen => (15, 0, 0, 0),
            Preset::Custom => {
                let (width, height, _) = RESOLUTIONS[self.resolution.min(RESOLUTIONS.len() - 1)];
                (FRAMERATES[self.fps.min(FRAMERATES.len() - 1)].0, width, height, BITRATES[self.bitrate.min(BITRATES.len() - 1)].0)
            }
        }
    }

    fn command(&self) -> Value {
        let (fps, width, height, bitrate) = self.limits();
        quality_command(Some(fps), Some(width), Some(height), Some(bitrate))
    }
}

/// Where the sound of the share comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioChoice {
    #[default]
    Computer,
    Program,
    /// The sound input of a capture card.
    Device,
    None,
}

/// What is known about the share that is running, for its settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Shared {
    /// The process of the window being shared, or zero when it is not a window.
    pub window_pid: u32,
    pub audio: AudioChoice,
    pub quality: Quality,
    /// True for the share of a connection with codes, false for the share in a call.
    pub manual: bool,
    /// True when a capture card is shared, which can send the sound of its own input.
    pub device_share: bool,
    /// The place of the sound input in the list of inputs the picker found.
    pub device: usize,
    /// The quality goes to the engine once it is ready, as `pendingQuality` did.
    pub pending_quality: bool,
}

/// A field with its name above and a list that opens, as `.field` with a `select` inside.
/// True when the pick changed.
pub(crate) fn select_field(ui: &mut Ui, palette: &Palette, label: &str, width: f32, options: &[&str], chosen: &mut usize) -> bool {
    let mut changed = false;
    ui.allocate_ui_with_layout(Vec2::new(width, 16.0 + 4.0 + 38.0), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(RichText::new(label).color(palette.muted).size(12.0));
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 38.0), Sense::click());
        ui.painter().rect_filled(rect, CornerRadius::same(4), palette.rail);
        let shown = options.get(*chosen).copied().unwrap_or("");
        widgets::paint_text_clipped(ui, rect.left_center() + Vec2::new(10.0, 0.0), rect.width() - 36.0, shown, FontId::proportional(15.0), palette.text);
        crate::icons::CHEVRON_DOWN.image(16.0, palette.soft).paint_at(ui, Rect::from_center_size(rect.right_center() - Vec2::new(14.0, 0.0), Vec2::splat(16.0)));
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        egui::Popup::menu(&response).width(rect.width()).show(|ui| {
            for (index, option) in options.iter().enumerate() {
                if crate::app::menu_item(ui, palette, option, false) && *chosen != index {
                    *chosen = index;
                    changed = true;
                }
            }
        });
    });
    changed
}

/// The four ways to set the quality and, for "Personalizada", the three lists, as
/// `.preset-grid` and `.custom-grid`.
pub(crate) fn quality_section(ui: &mut Ui, palette: &Palette, quality: &mut Quality, width: f32) {
    section_title(ui, palette, "QUALIDADE DA TRANSMISSÃO");
    let columns = (((width + 8.0) / (170.0 + 8.0)).floor() as usize).clamp(1, PRESETS.len());
    let card_width = (width - 8.0 * (columns - 1) as f32) / columns as f32;
    let spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
    for row in PRESETS.chunks(columns) {
        ui.horizontal(|ui| {
            for (preset, name, meta) in row {
                let (rect, response) = ui.allocate_exact_size(Vec2::new(card_width, 60.0), Sense::click());
                ui.painter().rect_filled(rect, CornerRadius::same(6), if response.hovered() { palette.hover } else { palette.sidebar });
                if quality.preset == *preset {
                    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(2.0, palette.accent), egui::StrokeKind::Inside);
                }
                let left = rect.left() + 14.0;
                widgets::paint_text_clipped(ui, egui::pos2(left, rect.top() + 12.0 + 9.0), rect.width() - 28.0, name, FontId::new(14.0, theme::semibold()), palette.strong);
                widgets::paint_text_clipped(ui, egui::pos2(left, rect.top() + 12.0 + 18.0 + 2.0 + 7.5), rect.width() - 28.0, meta, FontId::proportional(12.0), palette.muted);
                if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    quality.preset = *preset;
                }
            }
        });
    }
    if quality.preset == Preset::Custom {
        let field = (width - 16.0) / 3.0;
        ui.horizontal(|ui| {
            let names = |list: &[&'static str]| list.to_vec();
            select_field(ui, palette, "Resolução", field, &names(&RESOLUTIONS.map(|(_, _, name)| name)), &mut quality.resolution);
            select_field(ui, palette, "Quadros por segundo", field, &names(&FRAMERATES.map(|(_, name)| name)), &mut quality.fps);
            select_field(ui, palette, "Taxa máxima", field, &names(&BITRATES.map(|(_, name)| name)), &mut quality.bitrate);
        });
    }
    ui.spacing_mut().item_spacing = spacing;
}

/// One choice of `.segmented`: blue when picked, faded when it cannot be picked.
fn segment(ui: &mut Ui, palette: &Palette, label: &str, chosen: bool, enabled: bool) -> bool {
    let ink = if chosen { Color32::WHITE } else { palette.text };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(13.0), ink);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + 24.0, galley.size().y.max(16.0) + 14.0), sense);
    let fade = if enabled { 1.0 } else { 0.4 };
    ui.painter().rect_filled(rect, CornerRadius::same(4), if chosen { palette.accent } else { palette.sidebar }.gamma_multiply(fade));
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, ink.gamma_multiply(fade));
    if !enabled {
        return false;
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Where the sound comes from, as `buildAudio` and `writeAudio`: the program alone only for a
/// shared program, the input of the card only for a shared card. True when it changed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn audio_section(
    ui: &mut Ui,
    palette: &Palette,
    audio: &mut AudioChoice,
    device: &mut usize,
    process: bool,
    device_share: bool,
    inputs: &[(String, String)],
    width: f32,
) -> bool {
    let before = (*audio, *device);
    // What cannot be given falls back on the sound of the computer.
    if (*audio == AudioChoice::Program && !process) || (*audio == AudioChoice::Device && (!device_share || inputs.is_empty())) {
        *audio = AudioChoice::Computer;
    }
    section_title(ui, palette, "SOM DA TRANSMISSÃO");
    let spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(width);
        if segment(ui, palette, "Som do computador", *audio == AudioChoice::Computer, true) {
            *audio = AudioChoice::Computer;
        }
        if !device_share && segment(ui, palette, "Só o som do programa", *audio == AudioChoice::Program, process) {
            *audio = AudioChoice::Program;
        }
        if device_share && segment(ui, palette, "Som da placa", *audio == AudioChoice::Device, !inputs.is_empty()) {
            *audio = AudioChoice::Device;
        }
        if segment(ui, palette, "Sem som", *audio == AudioChoice::None, true) {
            *audio = AudioChoice::None;
        }
    });
    if *audio == AudioChoice::Device && !inputs.is_empty() {
        ui.add_space(2.0);
        let names: Vec<&str> = inputs.iter().map(|(_, name)| name.as_str()).collect();
        *device = (*device).min(names.len() - 1);
        select_field(ui, palette, "Entrada de som da placa", width, &names, device);
    }
    if !process && !device_share {
        ui.add_space(7.0);
        ui.add(egui::Label::new(RichText::new("Para mandar só o som de um programa, compartilhe um programa em Aplicativos.").color(palette.muted).size(13.0)).wrap());
    }
    ui.spacing_mut().item_spacing = spacing;
    (*audio, *device) != before
}

/// What the engine said about a change, in words for the person. Nothing for other events.
pub fn describe(event: &Value) -> Option<String> {
    let text = |name: &str| event.get(name).and_then(Value::as_str).unwrap_or("");
    let number = |name: &str| event.get(name).and_then(Value::as_i64).unwrap_or(0);
    match text("event") {
        "quality" => {
            let fps = if number("max_fps") > 0 { format!("até {} fps", number("max_fps")) } else { "até 60 fps".to_owned() };
            let bitrate = match number("max_bitrate_kbps") {
                0 => "taxa automática".to_owned(),
                kbps => format!("até {} Mbps", format!("{}", kbps as f64 / 1000.0).replace('.', ",")),
            };
            Some(format!("Qualidade aplicada: {}×{}, {fps}, {bitrate}.", number("width"), number("height")))
        }
        "audio" if !text("message").is_empty() => Some(format!("Som: {}.", text("message"))),
        "audio" => {
            let name = match text("scope") {
                "system" => "som do computador",
                "process" => "só o som do programa",
                "device" => "som da placa de captura",
                "none" => "sem som",
                other => other,
            };
            Some(format!("Som da transmissão: {name}."))
        }
        "command_failed" => Some(match text("command") {
            "switch_target" => "Não consegui trocar para essa tela, continuo mostrando a anterior.".to_owned(),
            "set_audio" => format!("Não consegui trocar o som ({}), continuo com o anterior.", text("message")),
            _ => format!("Não consegui mudar a qualidade ({}).", text("message")),
        }),
        _ => None,
    }
}

impl TelinhaApp {
    pub(crate) fn open_stream_settings(&mut self) {
        if self.room.shared.is_some() {
            self.room.stream_feedback.clear();
            self.room.stream_settings = true;
        }
    }

    /// Shows the settings of a made-up share. For the tests.
    pub fn stream_settings_for_test(&mut self, shared: Option<Shared>) {
        self.room.stream_settings = shared.is_some();
        self.room.shared = shared;
    }

    fn share_command(&self, manual: bool, command: &Value) -> bool {
        if manual { self.hub.manual_command(ManualRole::Share, command) } else { self.hub.stream_command(command) }
    }

    /// What the engine of a share said, kept to show in the window of its settings.
    pub(crate) fn stream_feedback(&mut self, event: &Value) {
        if let Some(text) = describe(event) {
            self.room.stream_feedback = text;
        }
        match event.get("event").and_then(Value::as_str) {
            // The quality picked with the share goes to the engine once it runs.
            Some("ready") => {
                let pending = self.room.shared.filter(|shared| shared.pending_quality);
                if let Some(shared) = pending {
                    self.share_command(shared.manual, &shared.quality.command());
                }
                if let Some(shared) = &mut self.room.shared {
                    shared.pending_quality = false;
                }
            }
            // The engine says which sound it ended up with, which may not be the one asked for.
            Some("audio") => {
                if let Some(shared) = &mut self.room.shared {
                    shared.audio = match event.get("scope").and_then(Value::as_str) {
                        Some("process") => AudioChoice::Program,
                        Some("device") => AudioChoice::Device,
                        Some("none") => AudioChoice::None,
                        _ => AudioChoice::Computer,
                    };
                }
            }
            _ => {}
        }
    }

    /// "Qualidade e som", as `#stream-dialog`: the changes hold at once, without dropping anyone.
    pub(crate) fn show_stream_settings(&mut self, context: &egui::Context) {
        let Some(mut shared) = self.room.shared.filter(|_| self.room.stream_settings) else {
            self.room.stream_settings = false;
            return;
        };
        let palette = self.palette;
        let before = shared;
        let inputs = self.room.audio_inputs.clone();
        let feedback = self.room.stream_feedback.clone();
        let mut close = false;
        let response = egui::Modal::new(egui::Id::unique("stream-settings")).frame(dialog_frame(&palette)).backdrop_color(Color32::from_black_alpha(178)).show(context, |ui| {
            ui.set_width(SETTINGS_WIDTH);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            if header(ui, &palette, "Qualidade e som") {
                close = true;
            }
            ui.add(egui::Label::new(RichText::new("As mudanças valem na hora, sem derrubar ninguém.").color(palette.muted).size(13.0)).wrap());
            quality_section(ui, &palette, &mut shared.quality, SETTINGS_WIDTH);
            audio_section(ui, &palette, &mut shared.audio, &mut shared.device, shared.window_pid > 0, shared.device_share, &inputs, SETTINGS_WIDTH);
            if !feedback.is_empty() {
                ui.add_space(13.0);
                ui.add(egui::Label::new(RichText::new(feedback.as_str()).color(palette.muted).size(13.0)).wrap());
            }
        });

        if shared.quality != before.quality {
            shared.pending_quality = false;
            if !self.share_command(shared.manual, &shared.quality.command()) {
                self.room.stream_feedback = "Não consegui mudar a qualidade agora.".to_owned();
            }
        }
        if (shared.audio, shared.device) != (before.audio, before.device) {
            let device = inputs.get(shared.device).map(|(id, _)| id.as_str());
            let audio = crate::share_picker::share_audio(shared.audio, shared.window_pid, device);
            if !self.share_command(shared.manual, &audio_command(&audio)) {
                self.room.stream_feedback = "Não consegui trocar o som agora.".to_owned();
            }
        }
        self.room.shared = Some(shared);
        if close || response.should_close() {
            self.room.stream_settings = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn each_choice_of_quality_asks_the_engine_for_its_limits() {
        assert_eq!(Quality::default().limits(), (0, 0, 0, 0));
        assert_eq!(Quality { preset: Preset::Games, ..Quality::default() }.limits(), (60, 2560, 1440, 0));
        assert_eq!(Quality { preset: Preset::Screen, ..Quality::default() }.limits(), (15, 0, 0, 0));
        let custom = Quality { preset: Preset::Custom, resolution: 2, fps: 3, bitrate: 3 };
        assert_eq!(custom.limits(), (30, 1920, 1080, 6000));
        let command = custom.command();
        assert_eq!((command["command"].as_str(), command["max_fps"].as_i64(), command["max_width"].as_i64()), (Some("set_quality"), Some(30), Some(1920)));
        // A place past the end of a list falls on its last entry.
        assert_eq!(Quality { preset: Preset::Custom, resolution: 99, fps: 99, bitrate: 99 }.limits(), (5, 854, 480, 1500));
    }

    #[test]
    fn what_the_engine_answers_is_put_in_words() {
        let quality = json!({ "event": "quality", "width": 1920, "height": 1080, "max_fps": 30, "max_bitrate_kbps": 1500 });
        assert_eq!(describe(&quality).unwrap(), "Qualidade aplicada: 1920×1080, até 30 fps, até 1,5 Mbps.");
        let automatic = json!({ "event": "quality", "width": 2560, "height": 1440, "max_fps": 0, "max_bitrate_kbps": 0 });
        assert_eq!(describe(&automatic).unwrap(), "Qualidade aplicada: 2560×1440, até 60 fps, taxa automática.");
        assert_eq!(describe(&json!({ "event": "audio", "scope": "process", "pid": 7 })).unwrap(), "Som da transmissão: só o som do programa.");
        assert_eq!(describe(&json!({ "event": "audio", "scope": "system", "message": "o programa não toca nada" })).unwrap(), "Som: o programa não toca nada.");
        assert_eq!(describe(&json!({ "event": "command_failed", "command": "set_audio", "message": "x" })).unwrap(), "Não consegui trocar o som (x), continuo com o anterior.");
        assert_eq!(describe(&json!({ "event": "stats" })), None);
    }
}
