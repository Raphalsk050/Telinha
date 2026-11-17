//! The quality and the sound of a share: picked with what to share, and changed while it runs
//! without stopping it, as the blocks `buildQuality` and `buildAudio` of share.js drew them.

use egui::{Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, Ui, Vec2};
use serde_json::{Value, json};
use telinha_core::hub::{ManualRole, audio_command, quality_command};

use crate::app::TelinhaApp;
use crate::dialogs::{dialog_frame, header, section_title};
use crate::theme::{self, Palette};
use crate::widgets;

const RESOLUTIONS: [(i64, i64, &str); 5] = [(0, 0, "Original"), (2560, 1440, "1440p"), (1920, 1080, "1080p"), (1280, 720, "720p"), (854, 480, "480p")];
const FRAMERATES: [(i64, &str); 6] = [(0, "Automático (até 60)"), (120, "120"), (60, "60"), (30, "30"), (15, "15"), (5, "5")];
/// The most kilobits per second. Zero leaves it to the network; the last line is a number the
/// person types.
const BITRATES: [(i64, &str); 15] = [
    (0, "Automática (sem limite)"),
    (50_000, "50 Mbps"),
    (40_000, "40 Mbps"),
    (30_000, "30 Mbps"),
    (20_000, "20 Mbps"),
    (15_000, "15 Mbps"),
    (10_000, "10 Mbps"),
    (8_000, "8 Mbps"),
    (6_000, "6 Mbps"),
    (4_000, "4 Mbps"),
    (3_000, "3 Mbps"),
    (2_000, "2 Mbps"),
    (1_500, "1,5 Mbps"),
    (1_000, "1 Mbps"),
    (-1, "Outro valor…"),
];
const TYPED_BITRATE: usize = BITRATES.len() - 1;
/// The most the engine spends on its own, when no limit is chosen.
const AUTOMATIC_CEILING_KBPS: f64 = 40_000.0;
const KEYFRAMES: [(i64, &str); 5] = [(0, "Automático"), (1_000, "1 s"), (2_000, "2 s"), (5_000, "5 s"), (10_000, "10 s")];
/// The width of the window of settings of a running share, as `.dialog`, less its padding.
const SETTINGS_WIDTH: f32 = 420.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    #[default]
    Auto,
    Games,
    Screen,
    Best,
    Weak,
    Custom,
}

const PRESETS: [(Preset, &str, &str); 6] = [
    (Preset::Auto, "Automática", "o Telinha ajusta sozinho"),
    (Preset::Games, "Jogos", "1440p · 60 fps · fluidez"),
    (Preset::Screen, "Compartilhamento de tela", "texto nítido · 15 fps"),
    (Preset::Best, "Máxima qualidade", "rede local · até 50 Mbps"),
    (Preset::Weak, "Internet fraca", "720p · 30 fps · até 3 Mbps"),
    (Preset::Custom, "Personalizada", "você escolhe tudo"),
];

/// How the encoder spends its bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RateMode {
    /// A steady rate that follows what the network carries.
    #[default]
    Auto,
    /// More bits where the picture moves, never past the most allowed.
    Variable,
    /// The same look in every picture, whatever rate it takes.
    Fixed,
}

const RATE_MODES: [(RateMode, &str, &str); 3] = [
    (RateMode::Auto, "Automática", "Taxa constante que acompanha a sua internet."),
    (RateMode::Variable, "Taxa variável", "Gasta mais nas cenas com movimento, sem passar da taxa máxima."),
    (RateMode::Fixed, "Qualidade fixa", "A imagem fica sempre igual; a taxa sobe quando a cena pede e não segue a internet."),
];

/// What gives way when the network is short.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Prefer {
    #[default]
    Balanced,
    /// Fewer frames, each one sharp: for text and still screens.
    Sharp,
    /// A smaller picture with every frame: for games and videos.
    Smooth,
}

const PREFERENCES: [(Prefer, &str); 3] = [(Prefer::Balanced, "Equilibrado"), (Prefer::Sharp, "Manter nitidez"), (Prefer::Smooth, "Manter fluidez")];

/// The limits the person put on the picture. The numbers of a custom choice are places in the
/// lists above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quality {
    pub preset: Preset,
    pub resolution: usize,
    pub fps: usize,
    pub bitrate: usize,
    /// The rate typed under "Outro valor…", in kilobits per second.
    pub typed_kbps: u32,
    pub mode: RateMode,
    /// How good the picture looks under "Qualidade fixa", from 0 to 100.
    pub image: u8,
    pub prefer: Prefer,
    pub keyframe: usize,
    pub low_latency: bool,
}

impl Default for Quality {
    fn default() -> Self {
        Self { preset: Preset::Auto, resolution: 0, fps: 0, bitrate: 0, typed_kbps: 12_000, mode: RateMode::Auto, image: 70, prefer: Prefer::Balanced, keyframe: 0, low_latency: true }
    }
}

/// What a choice of quality asks of the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encoding {
    /// The most frames per second, the largest width and height and the most kilobits per
    /// second. Zero leaves it to the engine.
    pub fps: i64,
    pub width: i64,
    pub height: i64,
    pub bitrate_kbps: i64,
    pub mode: RateMode,
    /// The quantizer of a fixed quality, 14 (best) to 40; zero for the other modes.
    pub quantizer: i64,
    pub prefer: Prefer,
    pub keyframe_ms: i64,
    pub low_latency: bool,
}

impl Quality {
    pub fn encoding(&self) -> Encoding {
        let auto = Encoding { fps: 0, width: 0, height: 0, bitrate_kbps: 0, mode: RateMode::Auto, quantizer: 0, prefer: Prefer::Balanced, keyframe_ms: 0, low_latency: true };
        match self.preset {
            Preset::Auto => auto,
            Preset::Games => Encoding { fps: 60, width: 2560, height: 1440, mode: RateMode::Variable, prefer: Prefer::Smooth, ..auto },
            Preset::Screen => Encoding { fps: 15, prefer: Prefer::Sharp, ..auto },
            Preset::Best => Encoding { fps: 60, bitrate_kbps: 50_000, mode: RateMode::Variable, ..auto },
            Preset::Weak => Encoding { fps: 30, width: 1280, height: 720, bitrate_kbps: 3_000, prefer: Prefer::Smooth, ..auto },
            Preset::Custom => {
                let (width, height, _) = RESOLUTIONS[self.resolution.min(RESOLUTIONS.len() - 1)];
                let bitrate = self.bitrate.min(BITRATES.len() - 1);
                let bitrate_kbps = if bitrate == TYPED_BITRATE { i64::from(self.typed_kbps.clamp(500, 100_000)) } else { BITRATES[bitrate].0 };
                Encoding {
                    fps: FRAMERATES[self.fps.min(FRAMERATES.len() - 1)].0,
                    width,
                    height,
                    bitrate_kbps,
                    mode: self.mode,
                    quantizer: if self.mode == RateMode::Fixed { 40 - (i64::from(self.image.min(100)) * 26 + 50) / 100 } else { 0 },
                    prefer: self.prefer,
                    keyframe_ms: KEYFRAMES[self.keyframe.min(KEYFRAMES.len() - 1)].0,
                    low_latency: self.low_latency,
                }
            }
        }
    }

    pub(crate) fn command(&self) -> Value {
        let encoding = self.encoding();
        let mut command = quality_command(Some(encoding.fps), Some(encoding.width), Some(encoding.height), Some(encoding.bitrate_kbps));
        command["rate_control"] = json!(match encoding.mode {
            RateMode::Auto => "auto",
            RateMode::Variable => "vbr",
            RateMode::Fixed => "quality",
        });
        command["quality"] = json!(encoding.quantizer);
        command["prefer"] = json!(match encoding.prefer {
            Prefer::Balanced => "balanced",
            Prefer::Sharp => "sharp",
            Prefer::Smooth => "smooth",
        });
        command["keyframe_ms"] = json!(encoding.keyframe_ms);
        command["latency"] = json!(if encoding.low_latency { "low" } else { "normal" });
        command
    }

    /// The choice as kept in `interface.json`, to be offered again next time.
    pub fn to_json(&self) -> Value {
        let preset = match self.preset {
            Preset::Auto => "auto",
            Preset::Games => "games",
            Preset::Screen => "screen",
            Preset::Best => "best",
            Preset::Weak => "weak",
            Preset::Custom => "custom",
        };
        json!({
            "preset": preset, "resolution": self.resolution, "fps": self.fps, "bitrate": self.bitrate, "typedKbps": self.typed_kbps,
            "mode": self.mode as u8, "image": self.image, "prefer": self.prefer as u8, "keyframe": self.keyframe, "lowLatency": self.low_latency,
        })
    }

    pub fn from_json(value: &Value) -> Self {
        let base = Self::default();
        let index = |name: &str| value.get(name).and_then(Value::as_u64).map(|number| number as usize);
        Self {
            preset: match value.get("preset").and_then(Value::as_str) {
                Some("games") => Preset::Games,
                Some("screen") => Preset::Screen,
                Some("best") => Preset::Best,
                Some("weak") => Preset::Weak,
                Some("custom") => Preset::Custom,
                _ => Preset::Auto,
            },
            resolution: index("resolution").unwrap_or(0).min(RESOLUTIONS.len() - 1),
            fps: index("fps").unwrap_or(0).min(FRAMERATES.len() - 1),
            bitrate: index("bitrate").unwrap_or(0).min(BITRATES.len() - 1),
            typed_kbps: value.get("typedKbps").and_then(Value::as_u64).map_or(base.typed_kbps, |kbps| (kbps as u32).clamp(500, 100_000)),
            mode: match index("mode") {
                Some(1) => RateMode::Variable,
                Some(2) => RateMode::Fixed,
                _ => RateMode::Auto,
            },
            image: index("image").map_or(base.image, |image| image.min(100) as u8),
            prefer: match index("prefer") {
                Some(1) => Prefer::Sharp,
                Some(2) => Prefer::Smooth,
                _ => Prefer::Balanced,
            },
            keyframe: index("keyframe").unwrap_or(0).min(KEYFRAMES.len() - 1),
            low_latency: value.get("lowLatency").and_then(Value::as_bool).unwrap_or(true),
        }
    }
}

/// What the choice costs the person sharing, in words: how much goes up for each person
/// watching, and a warning when it is more than the network carries now. The second value is
/// true for a warning.
pub fn cost_note(quality: &Quality, watching: usize, network_kbps: Option<f64>) -> (String, bool) {
    let encoding = quality.encoding();
    let mbps = |kbps: f64| {
        let value = kbps / 1000.0;
        if value >= 10.0 || value.fract() == 0.0 { format!("{value:.0}") } else { format!("{value:.1}").replace('.', ",") }
    };
    if encoding.mode == RateMode::Fixed {
        return ("Qualidade fixa não segue a internet: perto da qualidade máxima a taxa passa de 60 Mbps para cada pessoa assistindo, e numa internet que não aguenta a imagem trava.".to_owned(), true);
    }
    let mut text = if encoding.bitrate_kbps == 0 {
        format!("A taxa acompanha a sua internet, até {} Mbps de envio para cada pessoa assistindo.", mbps(AUTOMATIC_CEILING_KBPS))
    } else {
        format!("Até {} Mbps de envio para cada pessoa assistindo.", mbps(encoding.bitrate_kbps as f64))
    };
    if encoding.bitrate_kbps > 0 && watching > 1 {
        text.push_str(&format!(" Com {watching} pessoas, até {} Mbps no total.", mbps((encoding.bitrate_kbps * watching as i64) as f64)));
    }
    let wanted = if encoding.bitrate_kbps > 0 { encoding.bitrate_kbps as f64 } else { 0.0 };
    match network_kbps.filter(|network| *network > 0.0) {
        Some(network) if wanted > network * 1.1 => {
            text.push_str(&format!(" Sua internet aguenta cerca de {} Mbps agora: acima disso a imagem trava ou atrasa.", mbps(network)));
            (text, true)
        }
        _ => (text, false),
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

/// The ways to set the quality and, for "Personalizada", every choice, as `.preset-grid` and
/// `.custom-grid`. `watching` and `network_kbps` tell what the choice costs.
pub(crate) fn quality_section(ui: &mut Ui, palette: &Palette, quality: &mut Quality, width: f32, watching: usize, network_kbps: Option<f64>) {
    section_title(ui, palette, "QUALIDADE DA TRANSMISSÃO");
    let columns = (((width + 8.0) / (170.0 + 8.0)).floor() as usize).clamp(1, 3);
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
        custom_fields(ui, palette, quality, width);
    }
    let (note, warning) = cost_note(quality, watching, network_kbps);
    ui.add(egui::Label::new(RichText::new(note).color(if warning { palette.yellow } else { palette.muted }).size(12.5)).wrap());
    ui.spacing_mut().item_spacing = spacing;
}

/// Every choice of "Personalizada": the limits, the way of compressing, what gives way when the
/// network is short, and the advanced ones.
fn custom_fields(ui: &mut Ui, palette: &Palette, quality: &mut Quality, width: f32) {
    let names = |list: &[&'static str]| list.to_vec();
    // A fixed quality takes whatever rate each picture needs, so it has no most.
    let capped = quality.mode != RateMode::Fixed;
    let field = if capped { (width - 16.0) / 3.0 } else { (width - 8.0) / 2.0 };
    ui.horizontal(|ui| {
        select_field(ui, palette, "Resolução", field, &names(&RESOLUTIONS.map(|(_, _, name)| name)), &mut quality.resolution);
        select_field(ui, palette, "Quadros por segundo", field, &names(&FRAMERATES.map(|(_, name)| name)), &mut quality.fps);
        if capped {
            select_field(ui, palette, "Taxa máxima", field, &names(&BITRATES.map(|(_, name)| name)), &mut quality.bitrate);
        }
    });
    if capped && quality.bitrate == TYPED_BITRATE {
        let mut mbps = quality.typed_kbps as f32 / 1000.0;
        let shown = format!("{} Mbps", format!("{mbps:.1}").replace('.', ","));
        if crate::settings::slider_field(ui, palette, "Taxa máxima digitada", shown, &mut mbps, 0.5..=100.0, 0.5).changed() {
            quality.typed_kbps = (mbps * 1000.0).round() as u32;
        }
    }

    let mut mode = RATE_MODES.iter().position(|(mode, _, _)| *mode == quality.mode).unwrap_or(0);
    if select_field(ui, palette, "Compressão", width, &RATE_MODES.map(|(_, name, _)| name), &mut mode) {
        quality.mode = RATE_MODES[mode].0;
    }
    ui.add(egui::Label::new(RichText::new(RATE_MODES[mode].2).color(palette.muted).size(12.5)).wrap());
    if quality.mode == RateMode::Fixed {
        let mut image = f32::from(quality.image);
        let shown = match quality.image {
            0..=30 => format!("{}% · leve", quality.image),
            31..=75 => format!("{}% · boa", quality.image),
            _ => format!("{}% · máxima", quality.image),
        };
        if crate::settings::slider_field(ui, palette, "Qualidade da imagem", shown, &mut image, 0.0..=100.0, 1.0).changed() {
            quality.image = image.round() as u8;
        }
    }

    ui.add_space(4.0);
    ui.label(RichText::new("Quando a internet apertar").color(palette.muted).size(12.0));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        for (prefer, name) in PREFERENCES {
            if segment(ui, palette, name, quality.prefer == prefer, quality.mode != RateMode::Fixed) {
                quality.prefer = prefer;
            }
        }
    });

    egui::CollapsingHeader::new(RichText::new("Opções avançadas").color(palette.text).size(13.0)).id_salt("share-advanced").show(ui, |ui| {
        ui.horizontal(|ui| {
            select_field(ui, palette, "Quadro-chave a cada", (width - 16.0) / 2.0, &names(&KEYFRAMES.map(|(_, name)| name)), &mut quality.keyframe);
        });
        ui.add(egui::Label::new(RichText::new("Quadros-chave mais frequentes fazem a imagem voltar mais rápido depois de uma falha, e gastam mais taxa.").color(palette.muted).size(12.5)).wrap());
        ui.checkbox(&mut quality.low_latency, RichText::new("Baixa latência").color(palette.text).size(13.0));
        ui.add(egui::Label::new(RichText::new("Desligada, o codificador olha mais quadros antes de decidir: imagem um pouco melhor, um pouco mais de atraso.").color(palette.muted).size(12.5)).wrap());
    });
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
            let mode = match text("rate_control") {
                "vbr" => ", taxa variável",
                "quality" => ", qualidade fixa",
                _ => "",
            };
            let prefer = match text("prefer") {
                "sharp" => ", mantém a nitidez",
                "smooth" => ", mantém a fluidez",
                _ => "",
            };
            Some(format!("Qualidade aplicada: {}×{}, {fps}, {bitrate}{mode}{prefer}.", number("width"), number("height")))
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
            // What the network carries now, for the cost of a choice of quality.
            Some("stats") if event.get("role").and_then(Value::as_str) == Some("sender") => {
                self.room.share_network_kbps = event.get("bitrate_bps").and_then(Value::as_f64).filter(|bits| *bits > 0.0).map(|bits| bits / 1000.0);
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
        let watching = self.room.streams.outgoing.as_ref().map_or(0, |outgoing| outgoing.viewers.iter().filter(|(_, state)| state == "Connected").count());
        let network = self.room.share_network_kbps;
        let mut close = false;
        let response = crate::glass::modal(context, egui::Id::unique("stream-settings"), dialog_frame(&palette), |ui| {
            ui.set_width(SETTINGS_WIDTH);
            let top = ui.cursor().min.y;
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            if header(ui, &palette, "Qualidade e som") {
                close = true;
            }
            ui.add(egui::Label::new(RichText::new("As mudanças valem na hora, sem derrubar ninguém.").color(palette.muted).size(13.0)).wrap());
            // The choices scroll when "Personalizada" makes them taller than the window.
            let room = (context.content_rect().height() - 2.0 * crate::dialogs::DIALOG_MARGIN - (ui.cursor().min.y - top) - 40.0).max(160.0);
            egui::ScrollArea::vertical().id_salt("stream-settings-options").max_height(room).min_scrolled_height(room).show(ui, |ui| {
                quality_section(ui, &palette, &mut shared.quality, SETTINGS_WIDTH, watching, network);
                audio_section(ui, &palette, &mut shared.audio, &mut shared.device, shared.window_pid > 0, shared.device_share, &inputs, SETTINGS_WIDTH);
            });
            if !feedback.is_empty() {
                ui.add_space(13.0);
                ui.add(egui::Label::new(RichText::new(feedback.as_str()).color(palette.muted).size(13.0)).wrap());
            }
        });

        if shared.quality != before.quality {
            self.prefs.share_quality = Some(shared.quality.to_json());
            self.prefs.save();
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
        let limits = |quality: Quality| {
            let encoding = quality.encoding();
            (encoding.fps, encoding.width, encoding.height, encoding.bitrate_kbps)
        };
        assert_eq!(limits(Quality::default()), (0, 0, 0, 0));
        assert_eq!(limits(Quality { preset: Preset::Games, ..Quality::default() }), (60, 2560, 1440, 0));
        assert_eq!(limits(Quality { preset: Preset::Screen, ..Quality::default() }), (15, 0, 0, 0));
        assert_eq!(limits(Quality { preset: Preset::Weak, ..Quality::default() }), (30, 1280, 720, 3_000));
        let custom = Quality { preset: Preset::Custom, resolution: 2, fps: 3, bitrate: 8, ..Quality::default() };
        assert_eq!(limits(custom), (30, 1920, 1080, 6000));
        let command = custom.command();
        assert_eq!((command["command"].as_str(), command["max_fps"].as_i64(), command["max_width"].as_i64()), (Some("set_quality"), Some(30), Some(1920)));
        assert_eq!((command["rate_control"].as_str(), command["prefer"].as_str(), command["latency"].as_str()), (Some("auto"), Some("balanced"), Some("low")));
        // A place past the end of a list falls on its last entry; for the rate that is a typed one.
        let past = Quality { preset: Preset::Custom, resolution: 99, fps: 99, bitrate: 99, typed_kbps: 25_000, ..Quality::default() };
        assert_eq!(limits(past), (5, 854, 480, 25_000));
    }

    #[test]
    fn the_ways_of_compressing_reach_the_engine() {
        let fixed = Quality { preset: Preset::Custom, mode: RateMode::Fixed, image: 100, prefer: Prefer::Smooth, keyframe: 2, low_latency: false, ..Quality::default() };
        let command = fixed.command();
        assert_eq!(command["rate_control"].as_str(), Some("quality"));
        assert_eq!(command["quality"].as_i64(), Some(14));
        assert_eq!(command["prefer"].as_str(), Some("smooth"));
        assert_eq!(command["keyframe_ms"].as_i64(), Some(2_000));
        assert_eq!(command["latency"].as_str(), Some("normal"));
        assert_eq!(Quality { image: 0, ..fixed }.command()["quality"].as_i64(), Some(40));
        assert_eq!(Quality { mode: RateMode::Variable, ..fixed }.command()["quality"].as_i64(), Some(0));
        assert_eq!(Quality { preset: Preset::Games, ..Quality::default() }.command()["rate_control"].as_str(), Some("vbr"));
        assert_eq!(Quality { preset: Preset::Screen, ..Quality::default() }.command()["prefer"].as_str(), Some("sharp"));
        // The choice comes back the same from interface.json, and nonsense falls back.
        assert_eq!(Quality::from_json(&fixed.to_json()), fixed);
        assert_eq!(Quality::from_json(&json!({ "preset": 7, "bitrate": "x" })), Quality::default());
    }

    #[test]
    fn the_cost_of_a_choice_is_said_and_warned() {
        let ten = Quality { preset: Preset::Custom, bitrate: 6, ..Quality::default() };
        assert_eq!(cost_note(&ten, 3, None), ("Até 10 Mbps de envio para cada pessoa assistindo. Com 3 pessoas, até 30 Mbps no total.".to_owned(), false));
        let (text, warning) = cost_note(&ten, 1, Some(4_000.0));
        assert!(warning && text.ends_with("Sua internet aguenta cerca de 4 Mbps agora: acima disso a imagem trava ou atrasa."), "{text}");
        assert!(!cost_note(&Quality::default(), 2, Some(1_000.0)).1);
        assert!(cost_note(&Quality { mode: RateMode::Fixed, ..ten }, 1, None).1);
    }

    #[test]
    fn what_the_engine_answers_is_put_in_words() {
        let quality = json!({ "event": "quality", "width": 1920, "height": 1080, "max_fps": 30, "max_bitrate_kbps": 1500 });
        assert_eq!(describe(&quality).unwrap(), "Qualidade aplicada: 1920×1080, até 30 fps, até 1,5 Mbps.");
        let automatic = json!({ "event": "quality", "width": 2560, "height": 1440, "max_fps": 0, "max_bitrate_kbps": 0 });
        assert_eq!(describe(&automatic).unwrap(), "Qualidade aplicada: 2560×1440, até 60 fps, taxa automática.");
        let fixed = json!({ "event": "quality", "width": 1280, "height": 720, "max_fps": 30, "max_bitrate_kbps": 0, "rate_control": "quality", "prefer": "smooth" });
        assert_eq!(describe(&fixed).unwrap(), "Qualidade aplicada: 1280×720, até 30 fps, taxa automática, qualidade fixa, mantém a fluidez.");
        assert_eq!(describe(&json!({ "event": "audio", "scope": "process", "pid": 7 })).unwrap(), "Som da transmissão: só o som do programa.");
        assert_eq!(describe(&json!({ "event": "audio", "scope": "system", "message": "o programa não toca nada" })).unwrap(), "Som: o programa não toca nada.");
        assert_eq!(describe(&json!({ "event": "command_failed", "command": "set_audio", "message": "x" })).unwrap(), "Não consegui trocar o som (x), continuo com o anterior.");
        assert_eq!(describe(&json!({ "event": "stats" })), None);
    }
}
