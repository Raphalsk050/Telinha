//! The settings window: a sidebar of pages on the left, for who the person is, the voice and the
//! video, how the app looks, the messages and the updates, and the page picked on the right.

use egui::{Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};
use image::GenericImageView;
use telinha_core::avatars::MAX_AVATAR_BYTES;
use telinha_core::profile::{NameStyle, Style};

use crate::appearance::{Appearance, COLOR_THEMES, CustomTheme, Density, GlassLook, Gradient, MAX_CUSTOM_COLORS, MessageStyle, ThemeChoice};
use crate::dialogs::{Tone, button, text_field_hint};
use crate::name_style::{CARD_WIDTH, CardFace, card_spots, parse_color, write_color};
use crate::shortcuts::{Action, Shortcut};
use crate::theme::{self, Palette, ThemeName};
use crate::widgets;

/// What the person asked for in the settings.
pub enum SettingsAction {
    Name(String),
    /// A picture was picked, and the person now chooses which part of it to use.
    Crop { banner: bool, bytes: Vec<u8> },
    RemovePicture { banner: bool },
    /// Open "Selecione uma imagem", with the last avatars.
    AvatarPicker,
    /// How the app looks, as it is now being changed.
    Appearance(Box<Appearance>),
    /// The style of the profile, written the way the hub takes it.
    Style(serde_json::Value),
    /// The microphone and the speakers to use. None is the default of Windows.
    Devices { input: Option<String>, output: Option<String> },
    /// The input sensitivity: automatic, or where the gate opens, in decibels.
    Sensitivity { auto: bool, threshold: f32 },
    /// The icon of the window and the taskbar, by the id of `app_icons`.
    AppIcon(String),
    /// The camera to use, by name. None is the first one found.
    Camera(Option<String>),
    Blur(bool),
    /// Start or stop the test of the camera.
    Preview(bool),
    ShowLog,
    /// Look for a new version now.
    CheckUpdates,
    Notice(String),
    /// The keyboard shortcuts as they are now, to keep and to put in force.
    Shortcuts(Vec<Shortcut>),
    /// Start recording the keys of the shortcut at this place, or stop with None.
    Record(Option<usize>),
    /// How loud the sound effects play, in percent; `test` plays one to hear it.
    EffectsVolume { volume: f32, test: bool },
}

pub struct Settings {
    name: String,
    /// The microphones and the speakers plugged in when the window opened.
    inputs: Vec<String>,
    outputs: Vec<String>,
    cameras: Vec<String>,
    /// The style of the profile as the person is changing it, and as it was saved last.
    style: Style,
    saved_style: Style,
    /// When the person last tried to leave with changes not saved, to shake the bar.
    refused_at: Option<f64>,
    /// Whether leaving the page is refused now, and the clock of this frame.
    blocked: bool,
    now: f64,
    /// When the page shown was opened, for its entrance.
    page_opened: Option<f64>,
    /// The size while its slider is being dragged. It only takes hold when let go.
    zoom_drag: Option<f32>,
    /// How far down the page is scrolled, when a test wants a part further down.
    scroll_to: Option<f32>,
    page: Page,
}

/// What the window needs to know about the person and the app to draw itself.
pub struct SettingsView<'a> {
    pub name: &'a str,
    pub activity: &'a str,
    pub avatar: Option<egui::Image<'static>>,
    pub banner: Option<egui::Image<'static>>,
    pub appearance: &'a Appearance,
    pub version: &'a str,
    pub input: Option<&'a str>,
    pub output: Option<&'a str>,
    /// How loud the microphone is now, from 0 to 1, and the line under the bar.
    pub mic_level: f32,
    pub mic_status: &'a str,
    /// How loud the microphone is in decibels, whether that is sent now, and where the gate
    /// opens; then the sensitivity chosen.
    pub mic_db: f32,
    pub gate_open: bool,
    pub gate_threshold: f32,
    pub sensitivity_auto: bool,
    pub sensitivity_db: f32,
    pub app_icon: &'a str,
    pub camera: Option<&'a str>,
    pub blur: bool,
    /// Whether the camera is being tested, and its last frame.
    pub previewing: bool,
    pub preview: Option<egui::load::SizedTexture>,
    pub camera_status: &'a str,
    pub update: UpdateInfo,
    pub shortcuts: &'a [Shortcut],
    /// How loud the sound effects play, in percent.
    pub effects_volume: f32,
    /// The shortcut whose keys are being recorded, and the keys pressed so far.
    pub recording: Option<(usize, Vec<u32>)>,
    /// Why the shortcuts cannot work anywhere in Windows, when they cannot.
    pub shortcuts_problem: Option<&'a str>,
}

const EFFECTS: [(&str, &str, usize, usize); 5] = [("", "Nenhum", 0, 0), ("solid", "Sólido", 1, 1), ("gradient", "Degradê", 2, 2), ("neon", "Neon", 1, 1), ("prism", "Prisma", 2, 5)];
const DEFAULT_COLORS: [&str; 5] = ["#5865f2", "#eb459e", "#19b8b0", "#f0b232", "#23a55a"];
/// How long after the last change the style is saved and told to everyone.
/// The longest "about me", as the profile keeps it.
const BIO_LIMIT: usize = 190;
/// The width of what a page holds, and the room on each side of it.
/// The width of the content of a page, and the room at its right where the scroll bar runs.
const WIDTH: f32 = 656.0;
const GUTTER: f32 = 24.0;
const PAGE_PAD: f32 = 32.0;
/// The width of the cell of a theme dot: seven in a row fill the page.
const THEME_CELL: f32 = 93.0;
const SIDEBAR: f32 = 232.0;

/// The pages of the settings, each with a line in the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Page {
    #[default]
    Profile,
    Voice,
    Appearance,
    Messages,
    Shortcuts,
    Updates,
}

/// The lines of the sidebar, in their groups.
const PAGES: [(&str, Page, crate::icons::Icon, &str); 6] = [
    ("USUÁRIO", Page::Profile, crate::icons::PERSON, "Perfil"),
    ("APP", Page::Voice, crate::icons::MIC, "Voz e vídeo"),
    ("APP", Page::Appearance, crate::icons::PALETTE, "Aparência"),
    ("APP", Page::Messages, crate::icons::CHAT, "Mensagens"),
    ("APP", Page::Shortcuts, crate::icons::KEYBOARD, "Atalhos"),
    ("TELINHA", Page::Updates, crate::icons::REFRESH, "Atualizações"),
];

impl Page {
    fn title(self) -> &'static str {
        PAGES.iter().find(|(_, page, _, _)| *page == self).map_or("", |(_, _, _, name)| name)
    }
}

/// What is known about new versions, for the page of updates.
pub struct UpdateInfo {
    /// False when this copy of the app does not update itself.
    pub available: bool,
    /// True while a look or a download is under way, which holds the button.
    pub busy: bool,
    pub line: String,
}

/// One choice of a row of choices, lit when it is the one in force.
fn chip(ui: &mut Ui, palette: &Palette, label: &str, chosen: bool) -> bool {
    chip_button(ui, palette, label, chosen).clicked()
}

/// The same button, for who wants more than the click.
pub(crate) fn chip_button(ui: &mut Ui, palette: &Palette, label: &str, chosen: bool) -> egui::Response {
    let ink = if chosen { Color32::WHITE } else { palette.text };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::new(14.0, theme::semibold()), ink);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + 20.0, 32.0), Sense::click());
    let fill = if chosen { palette.accent } else if response.hovered() { palette.selected } else { palette.panel };
    ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small gray button, for what is not the main thing to do.
fn small_button(ui: &mut Ui, palette: &Palette, label: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::new(13.0, theme::semibold()), palette.strong);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + 20.0, 28.0), Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::same(4), if response.hovered() { palette.muted.gamma_multiply(0.55) } else { palette.selected });
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, palette.strong);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A box to tick with its sentence. True when it was clicked.
fn check(ui: &mut Ui, palette: &Palette, on: bool, label: &str) -> bool {
    // A switch at the right edge, as every on and off choice of Discord.
    ui.add_space(6.0);
    toggle_row(ui, palette, label, "", on)
}

/// A row as Discord draws its choices: the name in bold, a line under it that explains, and a
/// switch at the right edge that slides. True when it was clicked.
pub(crate) fn toggle_row(ui: &mut Ui, palette: &Palette, title: &str, note: &str, on: bool) -> bool {
    let width = ui.available_width().min(WIDTH);
    let text_width = width - 64.0;
    let title_galley = ui.painter().layout(title.to_owned(), FontId::new(15.0, theme::semibold()), palette.strong, text_width);
    let note_galley = (!note.is_empty()).then(|| ui.painter().layout(note.to_owned(), FontId::proportional(13.0), palette.muted, text_width));
    let height = title_galley.size().y + note_galley.as_ref().map_or(0.0, |galley| 4.0 + galley.size().y);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height.max(24.0)), Sense::click());
    ui.painter().galley(rect.left_top(), title_galley.clone(), palette.strong);
    if let Some(galley) = note_galley {
        ui.painter().galley(rect.left_top() + Vec2::new(0.0, title_galley.size().y + 4.0), galley, palette.muted);
    }
    let track = Rect::from_center_size(egui::pos2(rect.right() - 20.0, rect.top() + title_galley.size().y / 2.0 + 1.0), Vec2::new(40.0, 24.0));
    paint_switch(ui, palette, response.id, track, on, response.hovered());
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The switch of a [`toggle_row`]: the knob slides and the track takes the accent, eased.
pub(crate) fn paint_switch(ui: &Ui, palette: &Palette, id: egui::Id, track: Rect, on: bool, hovered: bool) {
    let t = crate::motion::ease(ui.ctx(), id.with("switch"), on, crate::motion::STATE);
    let lit = crate::motion::hover(ui.ctx(), id, hovered);
    let off = if palette.dark { Color32::from_rgb(0x4e, 0x50, 0x58) } else { Color32::from_rgb(0x80, 0x84, 0x8e) };
    let fill = crate::motion::mix(off, palette.accent, t);
    ui.painter().rect_filled(track, CornerRadius::same(12), crate::motion::mix(fill, Color32::WHITE, 0.08 * lit));
    let x = crate::motion::lerp(track.left() + 12.0, track.right() - 12.0, t);
    ui.painter().circle_filled(egui::pos2(x, track.center().y), 9.0, Color32::WHITE);
}

/// The bar of the input sensitivity, as Discord draws it: yellow up to where the gate opens and
/// green after it, with how loud the microphone is now lit over it. With `manual` the mark can be
/// dragged, and the new place comes back, in decibels.
fn sensitivity_bar(ui: &mut Ui, palette: &Palette, view: &SettingsView) -> Option<f32> {
    let width = ui.available_width().min(WIDTH);
    let (slot, response) = ui.allocate_exact_size(Vec2::new(width, 24.0), if view.sensitivity_auto { Sense::hover() } else { Sense::click_and_drag() });
    // Level with the text above it; the knob may stand out of the ends.
    let bar = Rect::from_center_size(slot.center(), Vec2::new(width - 4.0, 8.0));
    let (low, high) = (*telinha_voice::audio::SENSITIVITY_RANGE.start(), *telinha_voice::audio::SENSITIVITY_RANGE.end());
    let x_of = |db: f32| bar.left() + bar.width() * ((db - low) / (high - low)).clamp(0.0, 1.0);
    let threshold = if view.sensitivity_auto { view.gate_threshold } else { view.sensitivity_db };
    let mark = x_of(threshold);
    let yellow = Color32::from_rgb(0xf0, 0xb2, 0x32);
    let green = palette.green;
    let round = CornerRadius::same(4);
    let painter = ui.painter();
    if view.sensitivity_auto {
        // Automatic: the level alone, green while the voice goes out.
        painter.rect_filled(bar, round, if palette.glass { Color32::from_white_alpha(26) } else { palette.rail });
    } else {
        painter.with_clip_rect(Rect::from_min_max(bar.min, egui::pos2(mark, bar.max.y))).rect_filled(bar, round, yellow.gamma_multiply(0.35));
        painter.with_clip_rect(Rect::from_min_max(egui::pos2(mark, bar.min.y), bar.max)).rect_filled(bar, round, green.gamma_multiply(0.35));
    }
    let level = ui.ctx().animate_value_with_time(response.id.with("level"), x_of(view.mic_db), 0.08);
    if level > bar.left() + 0.5 {
        let lit = Rect::from_min_max(bar.min, egui::pos2(level, bar.max.y));
        if view.sensitivity_auto {
            let color = if view.gate_open { green } else { palette.muted };
            painter.with_clip_rect(lit).rect_filled(bar, round, color);
        } else {
            painter.with_clip_rect(lit.intersect(Rect::from_min_max(bar.min, egui::pos2(mark, bar.max.y)))).rect_filled(bar, round, yellow);
            painter.with_clip_rect(lit.intersect(Rect::from_min_max(egui::pos2(mark, bar.min.y), bar.max))).rect_filled(bar, round, green);
        }
    }
    if view.sensitivity_auto {
        return None;
    }
    let lit = crate::motion::hover(ui.ctx(), response.id, response.hovered() || response.dragged());
    let knob = egui::pos2(mark, bar.center().y);
    ui.painter().circle_filled(knob, crate::motion::lerp(8.0, 9.5, lit), Color32::WHITE);
    ui.painter().circle_stroke(knob, crate::motion::lerp(8.0, 9.5, lit), Stroke::new(1.0, Color32::from_black_alpha(40)));
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("{threshold:.0} dB"));
    if response.dragged() || response.clicked() {
        let x = response.interact_pointer_pos()?.x;
        let db = low + (high - low) * ((x - bar.left()) / bar.width()).clamp(0.0, 1.0);
        return Some(db.round());
    }
    None
}

/// An icon to pick, as Discord shows them: a rounded square that rises a little under the
/// mouse, with a ring and a check on the one in use. True when it was clicked.
fn app_icon_tile(ui: &mut Ui, palette: &Palette, id: &str, name: &str, bytes: &'static [u8], chosen: bool) -> bool {
    let (slot, response) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::click());
    let lit = crate::motion::hover(ui.ctx(), response.id, response.hovered());
    let picked = crate::motion::ease(ui.ctx(), response.id.with("picked"), chosen, crate::motion::STATE);
    let rect = Rect::from_center_size(slot.center() - Vec2::new(0.0, 2.0 * lit), Vec2::splat(crate::motion::lerp(48.0, 51.0, lit)));
    let round = CornerRadius::same(12);
    ui.painter().add(egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha((40.0 + 50.0 * lit) as u8) }.as_shape(rect, round));
    egui::Image::from_bytes(format!("bytes://app-icon/{id}.png"), bytes).corner_radius(round).paint_at(ui, rect);
    if picked > 0.0 {
        ui.painter().rect_stroke(rect.expand(3.0), CornerRadius::same(15), Stroke::new(2.0, palette.accent.gamma_multiply(picked)), egui::StrokeKind::Middle);
        let badge = egui::pos2(rect.right() + 1.0, rect.top() - 1.0);
        ui.painter().circle_filled(badge, 9.0 * picked, palette.accent);
        if picked > 0.6 {
            let tick = [badge + Vec2::new(-4.0, 0.0), badge + Vec2::new(-1.0, 3.0), badge + Vec2::new(4.0, -3.0)];
            ui.painter().add(egui::Shape::line(tick.to_vec(), Stroke::new(2.0, Color32::WHITE)));
        }
    }
    response.on_hover_text(name).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A square of a color that opens a picker, with a small cross to take the color away when it
/// may go. Returns whether the color changed and whether it was taken away.
fn swatch(ui: &mut Ui, palette: &Palette, color: &mut Color32, removable: bool) -> (bool, bool) {
    let (slot, _) = ui.allocate_exact_size(Vec2::new(44.0, 40.0), Sense::hover());
    let rect = Rect::from_min_size(slot.min + Vec2::new(0.0, 4.0), Vec2::splat(36.0));
    let response = ui.interact(rect, ui.scope_id().with(("swatch", slot.min.x as i32, slot.min.y as i32)), Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::same(8), palette.rail);
    ui.painter().rect_filled(rect.shrink(3.0), CornerRadius::same(5), *color);
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut changed = false;
    egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        changed = egui::color_picker::color_picker_color32(ui, color, egui::color_picker::Alpha::Opaque);
    });
    let mut removed = false;
    if removable {
        let badge = Rect::from_center_size(rect.right_top() + Vec2::new(-1.0, 1.0), Vec2::splat(18.0));
        let cross = ui.interact(badge, ui.scope_id().with(("swatch-remove", slot.min.x as i32, slot.min.y as i32)), Sense::click());
        ui.painter().circle_filled(badge.center(), 9.0, if cross.hovered() { palette.danger_hover } else { palette.danger });
        ui.painter().text(badge.center() - Vec2::new(0.0, 1.0), egui::Align2::CENTER_CENTER, "×", FontId::new(13.0, theme::bold()), Color32::WHITE);
        removed = cross.on_hover_text("Tirar esta cor").on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    }
    (changed, removed)
}

/// A field with its name above and a list that opens to pick from. Returns the pick: the inner
/// None stands for the default of Windows.
fn device_field(ui: &mut Ui, palette: &Palette, label: &str, width: f32, names: &[String], chosen: Option<&str>, default: &str) -> Option<Option<String>> {
    let mut picked = None;
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(RichText::new(label).color(palette.soft).size(13.0));
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 38.0), Sense::click());
        ui.painter().rect_filled(rect, CornerRadius::same(4), palette.rail);
        // A device that was picked and unplugged still shows, so the choice is not lost.
        let shown = chosen.unwrap_or(default);
        widgets::paint_text_clipped(ui, rect.left_center() + Vec2::new(10.0, 0.0), rect.width() - 40.0, shown, FontId::proportional(15.0), palette.text);
        crate::icons::CHEVRON_DOWN.image(16.0, palette.soft).paint_at(ui, Rect::from_center_size(rect.right_center() - Vec2::new(16.0, 0.0), Vec2::splat(16.0)));
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        egui::Popup::menu(&response).width(rect.width()).show(|ui| {
            if crate::app::menu_item(ui, palette, default, false) {
                picked = Some(None);
            }
            for name in names {
                if crate::app::menu_item(ui, palette, name, false) {
                    picked = Some(Some(name.clone()));
                }
            }
        });
    });
    picked
}

/// A round button that picks a theme, with its name under it. `paint` fills the circle.
fn theme_dot(ui: &mut Ui, palette: &Palette, label: &str, chosen: bool, paint: impl FnOnce(&egui::Painter, egui::Pos2, f32)) -> bool {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(13.0), if chosen { palette.strong } else { palette.soft });
    // Every dot takes the same cell, so the rows line up as columns.
    let (rect, response) = ui.allocate_exact_size(Vec2::new(THEME_CELL, 84.0), Sense::click());
    let center = egui::pos2(rect.center().x, rect.top() + 28.0);
    if chosen {
        ui.painter().circle_stroke(center, 29.0, Stroke::new(2.0, palette.accent));
    } else if response.hovered() {
        ui.painter().circle_stroke(center, 29.0, Stroke::new(2.0, palette.line_soft));
    }
    paint(ui.painter(), center, 25.0);
    ui.painter().circle_stroke(center, 25.0, Stroke::new(1.0, palette.line_soft));
    ui.painter().galley(egui::pos2(rect.center().x - galley.size().x / 2.0, rect.top() + 62.0), galley, palette.soft);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A slider with its name on the left and its value on the right, above it.
pub(crate) fn slider_field(ui: &mut Ui, palette: &Palette, label: &str, shown: String, value: &mut f32, range: std::ops::RangeInclusive<f32>, step: f64) -> egui::Response {
    ui.add_space(10.0);
    let mut name = None;
    ui.horizontal(|ui| {
        name = Some(ui.label(RichText::new(label).color(palette.strong).size(15.0).family(theme::semibold())));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(shown).color(palette.muted).size(13.0));
        });
    });
    ui.add_space(4.0);
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = ui.available_width();
        slider_look(ui, palette);
        let slider = ui.add(egui::Slider::new(value, range).step_by(step).show_value(false).trailing_fill(true));
        match name {
            Some(name) => slider.labelled_by(name.id),
            None => slider,
        }
    })
    .inner
}

/// Cuts the middle of a picture to the wanted shape and shrinks it until it is light enough to
/// go around in the presence.
pub fn prepare_picture(bytes: &[u8], size: (u32, u32)) -> Result<Vec<u8>, String> {
    let picture = image::load_from_memory(bytes).map_err(|_| "Não consegui abrir essa imagem. Tente PNG, JPG, WebP ou GIF.")?;
    let (width, height) = picture.dimensions();
    let shape = size.0 as f32 / size.1 as f32;
    let (crop_width, crop_height) = if width as f32 / height as f32 > shape {
        ((height as f32 * shape).round() as u32, height)
    } else {
        (width, (width as f32 / shape).round() as u32)
    };
    let cropped = picture.crop_imm((width - crop_width) / 2, (height - crop_height) / 2, crop_width.max(1), crop_height.max(1));
    encode_picture(&cropped, size)
}

/// Brings a picture already cut to its shape to the size of a profile picture, as light as the
/// presence takes.
pub fn encode_picture(cropped: &image::DynamicImage, size: (u32, u32)) -> Result<Vec<u8>, String> {
    let resized = cropped.resize_exact(size.0, size.1, image::imageops::FilterType::Lanczos3).to_rgba8();
    let encoder = webp::Encoder::from_rgba(resized.as_raw(), size.0, size.1);
    for quality in [90.0, 80.0, 65.0, 50.0] {
        let encoded = encoder.encode(quality);
        if encoded.len() <= MAX_AVATAR_BYTES {
            return Ok(encoded.to_vec());
        }
    }
    Err("A imagem ficou grande demais, tente outra.".into())
}

fn pick_picture(banner: bool, actions: &mut Vec<SettingsAction>) {
    match pick_picture_file() {
        Some(Ok(bytes)) => actions.push(SettingsAction::Crop { banner, bytes }),
        Some(Err(text)) => actions.push(SettingsAction::Notice(text)),
        None => {}
    }
}

/// Asks for a picture on the disk. None when the person gave up.
pub(crate) fn pick_picture_file() -> Option<Result<Vec<u8>, String>> {
    let path = rfd::FileDialog::new().add_filter("Imagens", &["png", "jpg", "jpeg", "webp", "gif"]).pick_file()?;
    Some(std::fs::read(&path).map_err(|_| "Não consegui ler esse arquivo.".to_owned()))
}

/// A color nobody picked, dark or light as the base of the theme asks.
fn random_color(light: bool) -> Color32 {
    let pick = |range: u32| telinha_core::text::random_bytes::<4>().iter().fold(0u32, |sum, byte| sum * 256 + u32::from(*byte)) % range;
    let hue = pick(360) as f32;
    let saturation = (55 + pick(35)) as f32 / 100.0;
    let lightness = if light { 74 + pick(14) } else { 22 + pick(28) } as f32 / 100.0;
    // From hue, saturation and lightness to red, green and blue.
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let part = |shift: f32| {
        let turn = (shift + hue / 30.0) % 12.0;
        lightness - chroma / 2.0 * (turn - 3.0).min(9.0 - turn).clamp(-1.0, 1.0)
    };
    Color32::from_rgb((part(0.0) * 255.0).round() as u8, (part(8.0) * 255.0).round() as u8, (part(4.0) * 255.0).round() as u8)
}

impl Settings {
    pub fn new(name: &str, inputs: Vec<String>, outputs: Vec<String>, cameras: Vec<String>, style: Style) -> Self {
        Self {
            name: name.to_owned(),
            inputs,
            outputs,
            cameras,
            saved_style: style.clone(),
            style,
            refused_at: None,
            blocked: false,
            now: 0.0,
            page_opened: None,
            zoom_drag: None,
            scroll_to: None,
            page: Page::default(),
        }
    }

    /// Shows this page, from its top. For the tests.
    pub fn page_for_test(&mut self, page: Page) {
        self.page = page;
        self.scroll_to = None;
    }

    pub fn page(&self) -> Page {
        self.page
    }

    /// Changes the name and the pronouns without saving, or undoes the changes. For the tests.
    pub fn profile_draft_for_test(&mut self, draft: Option<(&str, &str)>) {
        match draft {
            Some((name, pronouns)) => {
                self.name = name.to_owned();
                self.style.pronouns = pronouns.to_owned();
            }
            None => self.style = self.saved_style.clone(),
        }
    }

    /// Shows the window scrolled this far down. For the tests.
    pub fn scroll_for_test(&mut self, offset: f32) {
        self.scroll_to = Some(offset);
    }

    /// Whether the name or the style were changed and not saved yet.
    fn unsaved(&self, saved_name: &str) -> bool {
        let name = self.name.trim();
        (!name.is_empty() && name != saved_name) || self.style != self.saved_style
    }

    /// Saves the name and the style as they are now.
    fn save_profile(&mut self, saved_name: &str, actions: &mut Vec<SettingsAction>) {
        let name = self.name.trim().to_owned();
        if !name.is_empty() && name != saved_name {
            actions.push(SettingsAction::Name(name));
        }
        if self.style != self.saved_style {
            if let Ok(style) = serde_json::to_value(&self.style) {
                actions.push(SettingsAction::Style(style));
            }
            self.saved_style = self.style.clone();
        }
    }

    /// The person tried to leave with changes not saved: the bar shakes and goes red, as Discord.
    fn refuse_to_leave(&mut self, now: f64) {
        self.refused_at = Some(now);
    }

    /// The form of the profile, in the column at the left of the preview: the name, the
    /// pronouns, the pictures, the style of the name, the colors of the card and the "about me".
    fn profile_form(&mut self, ui: &mut Ui, palette: &Palette, view: &SettingsView<'_>, actions: &mut Vec<SettingsAction>) {
        let width = ui.available_width();
        let label = field_title(ui, palette, "Nome de exibição", true);
        ui.allocate_ui(Vec2::new(width, 38.0), |ui| text_field_hint(ui, palette, &mut self.name, 32, view.name).labelled_by(label.id));

        let label = field_title(ui, palette, "Pronomes", false);
        ui.allocate_ui(Vec2::new(width, 38.0), |ui| text_field_hint(ui, palette, &mut self.style.pronouns, 40, "Adicione seus pronomes").labelled_by(label.id));

        field_title(ui, palette, "Avatar", false);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if button(ui, palette, "Mudar avatar", Tone::Primary).clicked() {
                actions.push(SettingsAction::AvatarPicker);
            }
            if view.avatar.is_some() && button(ui, palette, "Remover avatar", Tone::Ghost).clicked() {
                actions.push(SettingsAction::RemovePicture { banner: false });
            }
        });

        field_title(ui, palette, "Banner do perfil", false);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if button(ui, palette, "Mudar banner", Tone::Primary).clicked() {
                pick_picture(true, actions);
            }
            if view.banner.is_some() && button(ui, palette, "Remover banner", Tone::Ghost).clicked() {
                actions.push(SettingsAction::RemovePicture { banner: true });
            }
        });

        field_title(ui, palette, "Estilo do nome", false);
        let current = self.style.name.as_ref().map_or("", |name| name.effect.as_str()).to_owned();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 6.0);
            for (effect, label, least, most) in EFFECTS {
                if chip(ui, palette, label, current == effect) && current != effect {
                    self.style.name = (!effect.is_empty()).then(|| {
                        let before = self.style.name.as_ref().map_or_else(Vec::new, |name| name.colors.clone());
                        let count = before.len().clamp(least, most);
                        let colors = before.into_iter().chain(DEFAULT_COLORS.iter().map(|color| (*color).to_owned())).take(count).collect();
                        NameStyle { effect: effect.to_owned(), colors }
                    });
                }
            }
        });
        if let Some(name) = &mut self.style.name {
            ui.add_space(10.0);
            let (least, most) = EFFECTS.iter().find(|(effect, ..)| *effect == name.effect).map_or((1, 1), |(_, _, least, most)| (*least, *most));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                let removable = name.colors.len() > least;
                let mut remove = None;
                for (index, text) in name.colors.iter_mut().enumerate() {
                    let mut color = parse_color(text).unwrap_or(Color32::GRAY);
                    let (edited, removed) = swatch(ui, palette, &mut color, removable);
                    if edited {
                        *text = write_color(color);
                    }
                    if removed {
                        remove = Some(index);
                    }
                }
                if let Some(index) = remove {
                    name.colors.remove(index);
                }
                if name.colors.len() < most && small_button(ui, palette, "Adicionar cor") {
                    name.colors.push(DEFAULT_COLORS[name.colors.len() % DEFAULT_COLORS.len()].to_owned());
                }
            });
        }

        ui.add_space(30.0);
        let themed = self.style.theme.is_some();
        if toggle_row(ui, palette, "Cor do perfil", "Duas cores fazem a moldura e o fundo do seu cartão.", themed) {
            self.style.theme = (!themed).then(|| DEFAULT_COLORS[..2].iter().map(|color| (*color).to_owned()).collect());
        }
        if let Some(colors) = &mut self.style.theme {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 20.0;
                for (text, label) in colors.iter_mut().zip(["Principal", "Destaque"]) {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        let mut color = parse_color(text).unwrap_or(Color32::GRAY);
                        if swatch(ui, palette, &mut color, false).0 {
                            *text = write_color(color);
                        }
                        ui.label(RichText::new(label).color(palette.muted).size(12.0));
                    });
                }
            });
        }

        let label = field_title(ui, palette, "Sobre mim", false);
        ui.label(RichText::new("Conte um pouco de você. Até 190 caracteres e 6 linhas.").color(palette.muted).size(13.0));
        ui.add_space(8.0);
        let fill = if palette.glass { Color32::from_white_alpha(12) } else { palette.rail };
        let about = Frame::new().fill(fill).corner_radius(CornerRadius::same(if palette.glass { 10 } else { 4 })).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.style.bio)
                    .frame(Frame::new())
                    .font(FontId::proportional(15.0))
                    .text_color(palette.text)
                    .hint_text(RichText::new("Sobre mim").color(palette.muted))
                    .char_limit(BIO_LIMIT)
                    .desired_rows(4)
                    .desired_width(ui.available_width()),
            )
            .labelled_by(label.id)
        });
        if about.inner.has_focus() {
            ui.painter().rect_stroke(about.response.rect, CornerRadius::same(if palette.glass { 10 } else { 4 }), Stroke::new(2.0, palette.accent), egui::StrokeKind::Outside);
        }
        let used = self.style.bio.chars().count();
        ui.add_space(4.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let ink = if used + 10 >= BIO_LIMIT { palette.yellow } else { palette.muted };
            ui.label(RichText::new(format!("{used}/{BIO_LIMIT}")).color(ink).size(12.0));
        });
        // Room for the bar of unsaved changes, which floats over the foot of the page.
        ui.add_space(72.0);
    }

    /// The card as the others will see it, beside the form and always in sight. Hovering its
    /// banner or its picture offers to change them, as Discord.
    fn profile_preview(&mut self, ui: &mut Ui, palette: &Palette, view: &SettingsView<'_>, actions: &mut Vec<SettingsAction>) {
        // It rises into place when the page opens.
        let now = ui.input(|input| input.time);
        let opened = *self.page_opened.get_or_insert(now);
        let shown = ((now - opened) / 0.35).clamp(0.0, 1.0) as f32;
        let shown = egui::emath::easing::cubic_out(shown);
        if shown < 1.0 {
            ui.ctx().request_repaint();
        }
        ui.multiply_opacity(shown);
        ui.add_space(8.0 + (1.0 - shown) * 14.0);
        ui.label(RichText::new("PRÉVIA").color(palette.muted).size(12.0).family(theme::bold()));
        ui.add_space(10.0);
        let name = if self.name.trim().is_empty() { view.name } else { self.name.trim() };
        let face = CardFace { name, activity: view.activity, style: Some(&self.style), avatar: view.avatar.clone(), banner: view.banner.clone(), online: true, edit: false, shadow: true };
        let spots = card_spots(ui, palette, face);
        let banner = ui.interact(spots.banner, ui.scope_id().with("preview-banner"), Sense::click());
        let portrait = ui.interact(spots.portrait, ui.scope_id().with("preview-portrait"), Sense::click());
        // The picture sits over the banner, so the banner is not lit while the picture is.
        let banner_lit = crate::motion::hover(ui.ctx(), banner.id, banner.hovered() && !portrait.hovered());
        let portrait_lit = crate::motion::hover(ui.ctx(), portrait.id, portrait.hovered());
        if banner_lit > 0.0 {
            ui.painter().rect_filled(spots.banner, CornerRadius { nw: 11, ne: 11, sw: 0, se: 0 }, Color32::from_black_alpha((120.0 * banner_lit) as u8));
            let at = egui::pos2(spots.banner.right() - 12.0, spots.banner.top() + 12.0);
            ui.painter().text(at, egui::Align2::RIGHT_TOP, "Mudar banner", FontId::new(12.0, theme::bold()), Color32::WHITE.gamma_multiply(banner_lit));
        }
        if portrait_lit > 0.0 {
            ui.painter().circle_filled(spots.portrait.center(), spots.portrait.width() / 2.0, Color32::from_black_alpha((140.0 * portrait_lit) as u8));
            ui.painter().text(spots.portrait.center(), egui::Align2::CENTER_CENTER, "MUDAR", FontId::new(11.0, theme::bold()), Color32::WHITE.gamma_multiply(portrait_lit));
        }
        if portrait.on_hover_text("Mudar avatar").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            actions.push(SettingsAction::AvatarPicker);
        } else if banner.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            pick_picture(true, actions);
        }
        ui.add_space(12.0);
        ui.add(egui::Label::new(RichText::new("Quem estiver em uma conversa com você vê seu nome e seu cartão assim.").color(palette.muted).size(12.0)).wrap());
    }

    /// The bar that rises from the foot of the page while there are changes not saved, with the
    /// way to undo them and to save them. It shakes and goes red when the person tries to leave.
    fn unsaved_bar(&mut self, ui: &mut Ui, palette: &Palette, page: Rect, view: &SettingsView<'_>, actions: &mut Vec<SettingsAction>) {
        let pending = self.unsaved(view.name);
        let rise = crate::motion::ease(ui.ctx(), ui.scope_id().with("unsaved-bar"), pending, crate::motion::OPEN);
        if rise <= 0.0 {
            self.refused_at = None;
            return;
        }
        let now = ui.input(|input| input.time);
        let refused = self.refused_at.map_or(0.0, |at| (now - at) as f32);
        let (shake, alarm) = if self.refused_at.is_some() && refused < 0.9 {
            ui.ctx().request_repaint();
            let fade = 1.0 - refused / 0.9;
            ((refused * 48.0).sin() * 9.0 * fade.min(1.0), fade)
        } else {
            (0.0, 0.0)
        };
        let height = 56.0;
        let rect = Rect::from_min_size(egui::pos2(page.left() + shake, page.bottom() - 16.0 - height + (1.0 - rise) * (height + 24.0)), Vec2::new(page.width(), height));
        let painter = ui.painter().with_clip_rect(page.expand2(Vec2::new(12.0, 0.0)));
        let round = CornerRadius::same(if palette.glass { 14 } else { 8 });
        let base = if palette.glass { Color32::from_rgb(22, 23, 36) } else if palette.dark { Color32::from_rgb(17, 18, 20) } else { palette.float };
        let fill = crate::motion::mix(base, palette.danger, alarm).gamma_multiply(rise);
        painter.add(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha((110.0 * rise) as u8) }.as_shape(rect, round));
        painter.rect_filled(rect, round, fill);
        if palette.glass {
            painter.rect_stroke(rect, round, Stroke::new(1.0, crate::glass::EDGE.gamma_multiply(rise)), egui::StrokeKind::Inside);
        }
        let ink = if palette.dark || palette.glass || alarm > 0.0 { Color32::WHITE } else { palette.strong };
        painter.text(rect.left_center() + Vec2::new(18.0, 0.0), egui::Align2::LEFT_CENTER, "Cuidado — você tem alterações que não foram salvas!", FontId::new(14.0, theme::semibold()), ink.gamma_multiply(rise));

        let save_galley = painter.layout_no_wrap("Salvar alterações".to_owned(), FontId::new(14.0, theme::semibold()), Color32::WHITE);
        let save = Rect::from_min_size(egui::pos2(rect.right() - 12.0 - save_galley.size().x - 32.0, rect.center().y - 16.0), Vec2::new(save_galley.size().x + 32.0, 32.0));
        let save_response = ui.interact(save, ui.scope_id().with("unsaved-save"), Sense::click());
        let save_lit = crate::motion::hover(ui.ctx(), save_response.id, save_response.hovered());
        let green = Color32::from_rgb(0x24, 0x80, 0x46);
        painter.rect_filled(save, CornerRadius::same(if palette.glass { 10 } else { 4 }), crate::motion::mix(green, Color32::from_rgb(0x1a, 0x65, 0x34), save_lit).gamma_multiply(rise));
        painter.galley_with_override_text_color(save.center() - save_galley.size() / 2.0, save_galley, Color32::WHITE.gamma_multiply(rise));

        let undo_galley = painter.layout_no_wrap("Redefinir".to_owned(), FontId::new(14.0, theme::semibold()), ink);
        let undo = Rect::from_min_size(egui::pos2(save.left() - 12.0 - undo_galley.size().x - 16.0, rect.center().y - 16.0), Vec2::new(undo_galley.size().x + 16.0, 32.0));
        let undo_response = ui.interact(undo, ui.scope_id().with("unsaved-undo"), Sense::click());
        let underline = undo_response.hovered();
        let text_at = undo.center() - undo_galley.size() / 2.0;
        let text_width = undo_galley.size().x;
        let text_height = undo_galley.size().y;
        painter.galley_with_override_text_color(text_at, undo_galley, ink.gamma_multiply(rise));
        if underline {
            painter.hline(text_at.x..=text_at.x + text_width, text_at.y + text_height, Stroke::new(1.0, ink.gamma_multiply(rise)));
        }
        if !pending {
            return;
        }
        if save_response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.save_profile(view.name, actions);
            self.refused_at = None;
        }
        if undo_response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.name = view.name.to_owned();
            self.style = self.saved_style.clone();
            self.refused_at = None;
        }
    }

    /// The gradient the person makes themself: its base, its colors, its direction and how
    /// strong it shows.
    fn custom_editor(ui: &mut Ui, palette: &Palette, custom: &mut CustomTheme) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (label, light) in [("Base escura", false), ("Base clara", true)] {
                if chip(ui, palette, label, custom.light == light) && custom.light != light {
                    custom.light = light;
                    changed = true;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let removable = custom.colors.len() > 1;
            let mut remove = None;
            for (index, color) in custom.colors.iter_mut().enumerate() {
                let (edited, removed) = swatch(ui, palette, color, removable);
                changed |= edited;
                if removed {
                    remove = Some(index);
                }
            }
            if let Some(index) = remove {
                custom.colors.remove(index);
                changed = true;
            }
            if custom.colors.len() < MAX_CUSTOM_COLORS && small_button(ui, palette, "Adicionar cor") {
                custom.colors.push(custom.colors.last().copied().unwrap_or(Color32::GRAY));
                changed = true;
            }
            if small_button(ui, palette, "Me surpreenda") {
                let count = 2 + usize::from(telinha_core::text::random_bytes::<1>()[0] % 3);
                custom.colors = (0..count).map(|_| random_color(custom.light)).collect();
                custom.angle = f32::from(telinha_core::text::random_bytes::<1>()[0] % 72) * 5.0;
                changed = true;
            }
        });
        if custom.colors.len() > 1 {
            changed |= slider_field(ui, palette, "Direção do degradê", format!("{:.0}°", custom.angle), &mut custom.angle, 0.0..=360.0, 5.0).changed();
        }
        changed |= slider_field(ui, palette, "Intensidade das cores", format!("{:.0}%", custom.intensity), &mut custom.intensity, 0.0..=100.0, 5.0).changed();
        changed
    }

    /// Draws the window. The second value is false when the person closed it.
    pub fn show(&mut self, context: &egui::Context, palette: &Palette, view: SettingsView<'_>) -> (Vec<SettingsAction>, bool) {
        let mut actions = Vec::new();
        let mut open = true;
        let mut look = view.appearance.clone();
        self.now = context.input(|input| input.time);
        self.blocked = self.unsaved(view.name);
        let screen = context.content_rect();
        let height = (screen.height() - 48.0).clamp(400.0, 860.0);
        // On glass the window is a glass panel, and what is behind it goes dark and soft, as
        // `.overlay::before { background: rgba(5, 6, 14, .35); backdrop-filter: blur(6px) }`.
        let glass = crate::glass::look(context).filter(|_| palette.glass);
        let frame = if glass.is_some() { Frame::new() } else { Frame::new().fill(palette.main).corner_radius(CornerRadius::same(8)).stroke(Stroke::new(1.0, palette.line)) };
        let backdrop = if glass.is_some() { Color32::TRANSPARENT } else { Color32::from_black_alpha(178) };
        let response = egui::Modal::new(egui::Id::unique("settings")).frame(frame).backdrop_color(backdrop).show(context, |ui| {
            let kept = glass.map(|_| (crate::glass::reserve(ui.painter()), crate::glass::reserve(ui.painter())));
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.horizontal_top(|ui| {
                self.sidebar(ui, palette, &view, height);
                let (area, _) = ui.allocate_exact_size(Vec2::new(PAGE_PAD * 2.0 + WIDTH + GUTTER, height), Sense::hover());
                let inner = area.shrink2(Vec2::new(PAGE_PAD, 0.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)), |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(self.page.title()).color(palette.strong).size(20.0).family(theme::bold()));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if widgets::icon_button(ui, crate::icons::CLOSE, 30.0, palette.soft, palette.hover, "Fechar").clicked() {
                                open = false;
                            }
                        });
                    });
                    ui.add_space(4.0);
                    let room = ui.available_height() - 16.0;
                    let mut area = egui::ScrollArea::vertical().id_salt(("settings-page", self.page)).max_height(room).min_scrolled_height(room).auto_shrink([false, false]);
                    if let Some(offset) = self.scroll_to {
                        area = area.vertical_scroll_offset(offset);
                    }
                    if self.page == Page::Profile {
                        // The form scrolls at the left; the preview stays in sight at the right.
                        let page = Rect::from_min_size(ui.cursor().min, Vec2::new(inner.width(), room));
                        let form_width = page.width() - CARD_WIDTH - 28.0;
                        let form_rect = Rect::from_min_size(page.min, Vec2::new(form_width + 16.0, room));
                        let mut form = ui.new_child(egui::UiBuilder::new().max_rect(form_rect).layout(Layout::top_down(Align::Min)));
                        area.show(&mut form, |ui| {
                            ui.set_width(form_width);
                            self.profile_form(ui, palette, &view, &mut actions);
                        });
                        let preview_rect = Rect::from_min_size(egui::pos2(page.right() - CARD_WIDTH, page.top()), Vec2::new(CARD_WIDTH, room));
                        let mut preview = ui.new_child(egui::UiBuilder::new().max_rect(preview_rect).layout(Layout::top_down(Align::Min)));
                        self.profile_preview(&mut preview, palette, &view, &mut actions);
                        ui.allocate_rect(page, Sense::hover());
                        self.unsaved_bar(ui, palette, page.with_max_y(page.bottom() + 16.0), &view, &mut actions);
                        return;
                    }
                    area.show(ui, |ui| {
                        ui.set_width(WIDTH);
                        match self.page {
                            Page::Profile => {}
                            Page::Voice => {
                                section(ui, palette, "VOZ", true);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 12.0;
                                    let half = (WIDTH - 12.0) / 2.0;
                                    if let Some(input) = device_field(ui, palette, "Microfone", half, &self.inputs, view.input, "Microfone padrão do Windows") {
                                        actions.push(SettingsAction::Devices { input, output: view.output.map(str::to_owned) });
                                    }
                                    if let Some(output) = device_field(ui, palette, "Saída de áudio", half, &self.outputs, view.output, "Saída padrão do Windows") {
                                        actions.push(SettingsAction::Devices { input: view.input.map(str::to_owned), output });
                                    }
                                });
                                // The input sensitivity, as Discord lays it out: the switch of the automatic
                                // mode, then the bar where the gate opens with the voice lit over it.
                                ui.add_space(24.0);
                                if toggle_row(ui, palette, "Ajustar automaticamente a sensibilidade de entrada", "Controla quanto som o Telinha transmite do seu microfone.", view.sensitivity_auto) {
                                    actions.push(SettingsAction::Sensitivity { auto: !view.sensitivity_auto, threshold: view.sensitivity_db });
                                }
                                ui.add_space(12.0);
                                if let Some(threshold) = sensitivity_bar(ui, palette, &view) {
                                    actions.push(SettingsAction::Sensitivity { auto: false, threshold });
                                }
                                ui.add_space(8.0);
                                let note = if view.sensitivity_auto {
                                    "A barra fica verde quando a sua voz está sendo transmitida, e o anel verde em volta da sua foto acende junto."
                                } else {
                                    "Só o que passa da marca é transmitido: o amarelo fica de fora, o verde vai para a chamada e acende o anel em volta da sua foto."
                                };
                                ui.add(egui::Label::new(RichText::new(note).color(palette.muted).size(13.0)).wrap());
                                if !view.mic_status.is_empty() {
                                    ui.add_space(4.0);
                                    ui.add(egui::Label::new(RichText::new(view.mic_status).color(palette.muted).size(13.0)).wrap());
                                }

                                section(ui, palette, "EFEITOS SONOROS", false);
                                ui.add(egui::Label::new(RichText::new("Os sons de quando alguém entra ou sai, de silenciar e ensurdecer, de mensagens e de chamadas.").color(palette.muted).size(13.0)).wrap());
                                let mut volume = view.effects_volume;
                                let shown = if volume <= 0.0 { "Sem som".to_owned() } else { format!("{volume:.0}%") };
                                let slider = slider_field(ui, palette, "Volume dos efeitos", shown, &mut volume, 0.0..=200.0, 5.0);
                                if slider.changed() || slider.drag_stopped() {
                                    // A sound plays when the slider is let go, to hear how loud it came out.
                                    actions.push(SettingsAction::EffectsVolume { volume, test: slider.drag_stopped() || (slider.changed() && !slider.dragged()) });
                                }

                                section(ui, palette, "VÍDEO", false);
                                self.video_section(ui, palette, &view, &mut actions);

                            }
                            Page::Appearance => {
                                section(ui, palette, "TEMA", true);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    for (choice, label, theme) in [
                                        (ThemeChoice::Light, "Claro", Some(ThemeName::Light)),
                                        (ThemeChoice::Ash, "Cinza", Some(ThemeName::Gray)),
                                        (ThemeChoice::Dark, "Escuro", Some(ThemeName::Dark)),
                                        (ThemeChoice::Onyx, "Ônix", Some(ThemeName::Onyx)),
                                        (ThemeChoice::System, "Automático", None),
                                        (ThemeChoice::Glass, "Vidro", None),
                                    ] {
                                        let clicked = theme_dot(ui, palette, label, look.theme == choice, |painter, center, radius| match theme {
                                            // The wallpaper of the glass theme, small: its lights on the night.
                                            None if choice == ThemeChoice::Glass => {
                                                painter.circle_filled(center, radius, Color32::from_rgb(0x1b, 0x14, 0x46));
                                                for (offset, color) in [(Vec2::new(-9.0, -9.0), 0x7c3aed), (Vec2::new(10.0, -10.0), 0x06b6d4), (Vec2::new(9.0, 11.0), 0xdb2777), (Vec2::new(-10.0, 10.0), 0x2563eb)] {
                                                    let color = Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, color as u8);
                                                    let mut mesh = egui::Mesh::default();
                                                    let spot = center + offset;
                                                    mesh.colored_vertex(spot, color.gamma_multiply(0.9));
                                                    for step in 0..=36u32 {
                                                        let point = spot + Vec2::angled(std::f32::consts::TAU * step as f32 / 36.0) * 16.0;
                                                        // Kept inside the circle.
                                                        let away = point - center;
                                                        let point = center + away * (radius / away.length().max(radius));
                                                        mesh.colored_vertex(point, Color32::TRANSPARENT);
                                                    }
                                                    for step in 1..=36u32 {
                                                        mesh.add_triangle(0, step, step + 1);
                                                    }
                                                    painter.add(egui::Shape::mesh(mesh));
                                                }
                                                painter.circle_filled(center, radius * 0.55, Color32::from_rgba_unmultiplied(16, 18, 34, 168));
                                                painter.circle_stroke(center, radius * 0.55, Stroke::new(1.0, crate::glass::EDGE));
                                            }
                                            Some(theme) => {
                                                painter.circle_filled(center, radius, Palette::of(theme).main);
                                            }
                                            // Half light and half dark, cut along the diagonal.
                                            None => {
                                                painter.circle_filled(center, radius, Palette::of(ThemeName::Light).main);
                                                let dark: Vec<egui::Pos2> = (0..=20).map(|step| center + Vec2::angled((-45.0 + 9.0 * step as f32).to_radians()) * radius).collect();
                                                painter.add(egui::Shape::convex_polygon(dark, Palette::of(ThemeName::Dark).main, Stroke::NONE));
                                            }
                                        });
                                        if clicked {
                                            look.theme = choice;
                                        }
                                    }
                                });
                                if matches!(look.theme, ThemeChoice::Light | ThemeChoice::System) && check(ui, palette, look.dark_sidebar, "Manter a barra lateral escura no tema claro") {
                                    look.dark_sidebar = !look.dark_sidebar;
                                }
                                if look.theme == ThemeChoice::Glass {
                                    // The five knobs of the controls window of the sketch.
                                    if section_with(ui, palette, "VIDRO", false, Some("Restaurar padrão")).1 {
                                        look.glass = GlassLook::default();
                                    }
                                    card(ui, palette, |ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        for (index, (_, label, low, high, step)) in GlassLook::KNOBS.iter().enumerate() {
                                            let value = look.glass.knob(index);
                                            let shown = match index {
                                                2 => format!("{value:.0} px"),
                                                _ => format!("{value:.2}"),
                                            };
                                            slider_row(ui, palette, label, shown, value, *low..=*high, *step);
                                        }
                                    });
                                }

                                section(ui, palette, "TEMAS COLORIDOS", false);
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = Vec2::new(0.0, 6.0);
                                    let own = Gradient { light: look.custom.light, colors: look.custom.colors.clone(), angle: look.custom.angle, intensity: look.custom.intensity, motion: false };
                                    if theme_dot(ui, palette, "Personalizado", look.theme == ThemeChoice::Custom, |painter, center, radius| own.paint_dot(painter, center, radius)) {
                                        look.theme = ThemeChoice::Custom;
                                    }
                                    for (index, theme) in COLOR_THEMES.iter().enumerate() {
                                        let colors = theme.colors.iter().map(|color| Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, *color as u8)).collect();
                                        let gradient = Gradient { light: theme.light, colors, angle: theme.angle, intensity: 75.0, motion: false };
                                        if theme_dot(ui, palette, theme.name, look.theme == ThemeChoice::Color(index), |painter, center, radius| gradient.paint_dot(painter, center, radius)) {
                                            look.theme = ThemeChoice::Color(index);
                                        }
                                    }
                                });
                                if look.theme == ThemeChoice::Custom {
                                    Self::custom_editor(ui, palette, &mut look.custom);
                                }
                                if look.gradient().is_some() && check(ui, palette, look.motion, "Degradê em movimento") {
                                    look.motion = !look.motion;
                                }

                                section(ui, palette, "ÍCONE DO APLICATIVO", false);
                                ui.label(RichText::new("Muda o ícone da janela e da barra de tarefas do Telinha.").color(palette.muted).size(13.0));
                                ui.add_space(12.0);
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = Vec2::new(14.0, 14.0);
                                    for (id, name, bytes) in crate::app_icons::ICONS {
                                        if app_icon_tile(ui, palette, id, name, bytes, view.app_icon == id) && view.app_icon != id {
                                            actions.push(SettingsAction::AppIcon(id.to_owned()));
                                        }
                                    }
                                });

                                section(ui, palette, "DENSIDADE DA INTERFACE", false);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    for (density, _, label) in Density::ALL {
                                        if chip(ui, palette, label, look.density == density) {
                                            look.density = density;
                                        }
                                    }
                                });

                                section(ui, palette, "TAMANHO", false);
                                // The size only takes hold when the slider is let go: applying it in the middle
                                // of the drag would pull the slider from under the mouse.
                                let mut zoom = self.zoom_drag.unwrap_or(look.zoom);
                                let slider = slider_field(ui, palette, "Zoom", format!("{zoom:.0}%"), &mut zoom, 50.0..=200.0, 10.0);
                                if slider.dragged() {
                                    self.zoom_drag = Some(zoom);
                                } else if slider.drag_stopped() || slider.changed() {
                                    self.zoom_drag = None;
                                    look.zoom = zoom;
                                }

                                ui.add_space(8.0);
                                if small_button(ui, palette, "Voltar a aparência ao padrão") {
                                    look = Appearance::default();
                                    self.zoom_drag = None;
                                }

                            }
                            Page::Messages => {
                                section(ui, palette, "ESTILO DAS MENSAGENS", true);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    for (style, label) in [(MessageStyle::Cozy, "Confortável"), (MessageStyle::Compact, "Compacta")] {
                                        if chip(ui, palette, label, look.messages == style) {
                                            look.messages = style;
                                        }
                                    }
                                });
                                let font = format!("{} px", format!("{}", look.chat_font).replace('.', ","));
                                slider_field(ui, palette, "Tamanho da fonte do chat", font, &mut look.chat_font, 12.0..=24.0, 0.5);
                                slider_field(ui, palette, "Espaço entre grupos de mensagens", format!("{:.0} px", look.message_gap), &mut look.message_gap, 0.0..=24.0, 4.0);
                            }
                            Page::Shortcuts => shortcuts_page(ui, palette, &view, &mut actions),
                            Page::Updates => self.updates_page(ui, palette, &view, &mut actions),
                        }
                        ui.add_space(12.0);
                    });
                });
            });
            if let (Some(look), Some((veil, pane))) = (glass, kept) {
                let veil_pane = crate::glass::Pane { blur: Some(6.0), edge: false, shine: false, ..crate::glass::Pane::new(Color32::from_rgba_unmultiplied(5, 6, 14, 89), 0.0) };
                crate::glass::fill(ui.painter(), veil, screen, &veil_pane);
                crate::glass::fill(ui.painter(), pane, ui.min_rect(), &crate::glass::Pane::panel(look.main()));
            }
        });
        if response.should_close() {
            open = false;
        }
        // Changes not saved keep the window open until they are saved or undone, as Discord.
        if !open && self.unsaved(view.name) {
            open = true;
            self.refuse_to_leave(self.now);
        }
        if look != *view.appearance {
            actions.push(SettingsAction::Appearance(Box::new(look)));
        }
        (actions, open)
    }
}

impl Settings {
    /// Who the person is at the top, then the pages in their groups, then the version.
    fn sidebar(&mut self, ui: &mut Ui, palette: &Palette, view: &SettingsView<'_>, height: f32) {
        let (area, _) = ui.allocate_exact_size(Vec2::new(SIDEBAR, height), Sense::hover());
        // On glass the list of pages is a darker strip of the window, as `.settings-nav`.
        let (round, fill) = if palette.glass { (16, Color32::from_black_alpha(46)) } else { (8, palette.sidebar) };
        ui.painter().rect_filled(area, CornerRadius { nw: round, sw: round, ne: 0, se: 0 }, fill);
        let inner = area.shrink2(Vec2::new(12.0, 16.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)), |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 2.0);
            // The person, which opens the profile.
            let (card, response) = ui.allocate_exact_size(Vec2::new(inner.width(), 52.0), Sense::click());
            if response.hovered() {
                ui.painter().rect_filled(card, CornerRadius::same(6), palette.hover);
            }
            let picture = Rect::from_min_size(card.min + Vec2::new(6.0, 8.0), Vec2::splat(36.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(picture), |ui| widgets::avatar(ui, 36.0, view.name, view.avatar.clone()));
            let left = picture.right() + 10.0;
            widgets::paint_text_clipped(ui, egui::pos2(left, card.top() + 17.0), card.right() - left - 6.0, view.name, FontId::new(15.0, theme::semibold()), palette.strong);
            widgets::paint_text_clipped(ui, egui::pos2(left, card.top() + 35.0), card.right() - left - 6.0, "Editar perfil", FontId::proportional(12.5), palette.muted);
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.go_to(Page::Profile);
            }
            let mut group = "";
            for (name, page, icon, label) in PAGES {
                if name != group {
                    group = name;
                    ui.add_space(14.0);
                    let (line, _) = ui.allocate_exact_size(Vec2::new(inner.width(), 1.0), Sense::hover());
                    ui.painter().hline(line.x_range().shrink(6.0), line.center().y, Stroke::new(1.0, palette.line_soft));
                    ui.add_space(10.0);
                    ui.add(egui::Label::new(RichText::new(name).color(palette.soft).size(11.5).family(theme::bold())).selectable(false));
                    ui.add_space(4.0);
                }
                let chosen = self.page == page;
                let (row, response) = ui.allocate_exact_size(Vec2::new(inner.width(), 36.0), Sense::click());
                if chosen || response.hovered() {
                    ui.painter().rect_filled(row, CornerRadius::same(6), if chosen { palette.hover } else { palette.hover.gamma_multiply(0.6) });
                }
                let ink = if chosen { palette.strong } else { palette.text };
                icon.image(18.0, if chosen { palette.strong } else { palette.soft }).paint_at(ui, Rect::from_center_size(row.left_center() + Vec2::new(19.0, 0.0), Vec2::splat(18.0)));
                widgets::paint_text_clipped(ui, row.left_center() + Vec2::new(38.0, 0.0), row.width() - 44.0, label, FontId::new(14.5, if chosen { theme::semibold() } else { egui::FontFamily::Proportional }), ink);
                if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.go_to(page);
                }
            }
            ui.painter().text(egui::pos2(inner.left() + 6.0, inner.bottom()), egui::Align2::LEFT_BOTTOM, format!("Telinha {}", view.version), FontId::proportional(12.0), palette.muted);
        });
    }

    fn go_to(&mut self, page: Page) {
        if page == self.page {
            return;
        }
        if self.blocked {
            self.refuse_to_leave(self.now);
            return;
        }
        self.page = page;
        self.scroll_to = None;
        self.page_opened = None;
    }

    /// The version, the button that looks for a new one now, and the log of errors.
    fn updates_page(&self, ui: &mut Ui, palette: &Palette, view: &SettingsView<'_>, actions: &mut Vec<SettingsAction>) {
        section(ui, palette, "VERSÃO", true);
        ui.label(RichText::new(format!("Telinha {}", view.version)).color(palette.strong).size(16.0).family(theme::semibold()));
        ui.add_space(4.0);
        ui.add_enabled_ui(view.update.available && !view.update.busy, |ui| {
            let label = if view.update.busy { "Procurando…" } else { "Procurar atualizações" };
            if button(ui, palette, label, Tone::Primary).clicked() {
                actions.push(SettingsAction::CheckUpdates);
            }
        });
        if !view.update.line.is_empty() {
            ui.add(egui::Label::new(RichText::new(view.update.line.as_str()).color(palette.muted).size(13.0)).wrap());
        }

        section(ui, palette, "REGISTRO DE ERROS", false);
        ui.add(egui::Label::new(RichText::new("Quando algo dá errado, o Telinha escreve o que aconteceu num arquivo. Ele ajuda a descobrir a causa.").color(palette.muted).size(13.0)).wrap());
        ui.add_space(2.0);
        if button(ui, palette, "Abrir registro de erros", Tone::Plain).on_hover_text("Abre a pasta do arquivo que registra erros do app").clicked() {
            actions.push(SettingsAction::ShowLog);
        }
    }

    /// The preview of the camera on the left, its choices on the right, as `.camera-settings`.
    fn video_section(&self, ui: &mut Ui, palette: &Palette, view: &SettingsView<'_>, actions: &mut Vec<SettingsAction>) {
        let left = (WIDTH - 16.0) * 1.2 / 2.2;
        let right = WIDTH - 16.0 - left;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let (preview, _) = ui.allocate_exact_size(Vec2::new(left, left * 9.0 / 16.0), Sense::hover());
            ui.painter().rect_filled(preview, CornerRadius::same(8), palette.rail);
            match view.preview.filter(|_| view.previewing) {
                Some(texture) => crate::room_view::paint_video(ui, preview, texture, true, true),
                None => {
                    ui.painter().text(preview.center(), egui::Align2::CENTER_CENTER, "Prévia da câmera", FontId::proportional(13.0), palette.muted);
                }
            }
            ui.vertical(|ui| {
                ui.set_width(right);
                ui.spacing_mut().item_spacing.y = 10.0;
                if let Some(camera) = device_field(ui, palette, "Câmera", right, &self.cameras, view.camera, "Primeira câmera encontrada") {
                    actions.push(SettingsAction::Camera(camera));
                }
                let label = if view.previewing { "Parar teste" } else { "Testar câmera" };
                if button(ui, palette, label, Tone::Plain).clicked() {
                    actions.push(SettingsAction::Preview(!view.previewing));
                }
                if check(ui, palette, view.blur, "Desfocar o fundo") {
                    actions.push(SettingsAction::Blur(!view.blur));
                }
                if !view.camera_status.is_empty() {
                    ui.label(RichText::new(view.camera_status).color(palette.muted).size(13.0));
                }
            });
        });
    }
}

/// The keyboard shortcuts, as Discord lays them out: a row per shortcut with what it does, its
/// keys and a switch, and the button that adds one at the top.
fn shortcuts_page(ui: &mut Ui, palette: &Palette, view: &SettingsView<'_>, actions: &mut Vec<SettingsAction>) {
    let mut list = view.shortcuts.to_vec();
    let mut changed = false;
    ui.add_space(8.0);
    ui.horizontal_top(|ui| {
        ui.set_width(WIDTH);
        ui.vertical(|ui| {
            ui.set_width(WIDTH - 220.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(RichText::new("Atalhos do teclado").color(palette.strong).size(17.0).family(theme::semibold()));
            ui.add(egui::Label::new(RichText::new("Funcionam em qualquer lugar do Windows: com o Telinha minimizado, atrás de um jogo ou em outro app.").color(palette.muted).size(13.0)).wrap());
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if add_button(ui, palette) {
                list.push(Shortcut::new(Action::ToggleMute));
                changed = true;
                // The new one starts recording at once: nobody adds a shortcut to leave it empty.
                actions.push(SettingsAction::Record(Some(list.len() - 1)));
            }
        });
    });
    ui.add_space(20.0);
    let (note, tone) = match view.shortcuts_problem {
        Some(problem) => (problem, palette.danger),
        None => ("Clique em “Gravar atalho” e pressione as teclas. A gravação termina quando você soltar todas.", palette.accent),
    };
    info_banner(ui, palette, note, tone);
    ui.add_space(8.0);

    if list.is_empty() {
        ui.add_space(36.0);
        ui.vertical_centered(|ui| {
            ui.add(crate::icons::KEYBOARD.image(44.0, palette.muted));
            ui.add_space(12.0);
            ui.label(RichText::new("Nenhum atalho ainda").color(palette.strong).size(15.0).family(theme::semibold()));
            ui.add_space(4.0);
            ui.label(RichText::new("Adicione um para silenciar, ensurdecer ou falar com uma tecla.").color(palette.muted).size(13.0));
        });
        ui.add_space(20.0);
    }
    let mut remove = None;
    for index in 0..list.len() {
        let recording = view.recording.as_ref().filter(|(at, _)| *at == index).map(|(_, keys)| keys.as_slice());
        match shortcut_row(ui, palette, index, &list[index], recording) {
            RowAnswer::Action(action) => {
                list[index].action = action;
                changed = true;
            }
            RowAnswer::Toggle => {
                list[index].enabled = !list[index].enabled;
                changed = true;
            }
            RowAnswer::Record(on) => actions.push(SettingsAction::Record(on.then_some(index))),
            RowAnswer::Remove => remove = Some(index),
            RowAnswer::None => {}
        }
    }
    if let Some(index) = remove {
        list.remove(index);
        changed = true;
        if view.recording.is_some() {
            actions.push(SettingsAction::Record(None));
        }
    }

    ui.add_space(24.0);
    ui.add(
        egui::Label::new(RichText::new("Um jogo aberto como administrador pode esconder as teclas do Telinha enquanto estiver na frente. Nesse caso, abra o Telinha como administrador também.").color(palette.muted).size(12.5))
            .wrap(),
    );
    if changed {
        actions.push(SettingsAction::Shortcuts(list));
    }
}

/// What the person did on one row of the shortcuts.
enum RowAnswer {
    None,
    Action(Action),
    Toggle,
    Record(bool),
    Remove,
}

/// The width of the two columns of a row of shortcuts.
const ACTION_WIDTH: f32 = 236.0;
const KEYS_WIDTH: f32 = WIDTH - ACTION_WIDTH - 20.0 - 40.0 - 32.0;

/// One shortcut: "Ação" with its list, "Atalho" with the keys and the button that records them,
/// and the switch at the right edge. A round cross to remove it shows under the mouse.
fn shortcut_row(ui: &mut Ui, palette: &Palette, index: usize, shortcut: &Shortcut, recording: Option<&[u32]>) -> RowAnswer {
    let mut answer = RowAnswer::None;
    let left = ui.cursor().left();
    let top = ui.cursor().top();
    let fade = if shortcut.enabled || recording.is_some() { 1.0 } else { 0.5 };
    let round = CornerRadius::same(if palette.glass { 10 } else { 6 });
    let mut field = Rect::NOTHING;
    ui.add_space(20.0);
    ui.horizontal_top(|ui| {
        ui.set_width(WIDTH);
        ui.spacing_mut().item_spacing.x = 20.0;
        ui.vertical(|ui| {
            ui.set_width(ACTION_WIDTH);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.multiply_opacity(fade);
            ui.label(RichText::new("Ação").color(palette.soft).size(13.0).family(theme::semibold()));
            let (rect, response) = ui.allocate_exact_size(Vec2::new(ACTION_WIDTH, 40.0), Sense::click());
            let lit = crate::motion::hover(ui.ctx(), response.id, response.hovered());
            ui.painter().rect_filled(rect, round, field_fill(palette));
            ui.painter().rect_stroke(rect, round, Stroke::new(1.0, crate::motion::mix(palette.line_soft, palette.line, lit)), egui::StrokeKind::Inside);
            widgets::paint_text_clipped(ui, rect.left_center() + Vec2::new(12.0, 0.0), rect.width() - 44.0, shortcut.action.label(), FontId::proportional(14.5), palette.text);
            crate::icons::CHEVRON_DOWN.image(16.0, palette.soft).paint_at(ui, Rect::from_center_size(rect.right_center() - Vec2::new(18.0, 0.0), Vec2::splat(16.0)));
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            egui::Popup::menu(&response).width(rect.width()).show(|ui| {
                for action in Action::ALL {
                    if crate::app::menu_item(ui, palette, action.label(), false) && action != shortcut.action {
                        answer = RowAnswer::Action(action);
                    }
                }
            });
            let note = shortcut.action.note();
            if !note.is_empty() {
                ui.add(egui::Label::new(RichText::new(note).color(palette.muted).size(12.0)).wrap());
            }
        });
        ui.vertical(|ui| {
            ui.set_width(KEYS_WIDTH);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.multiply_opacity(fade);
            ui.label(RichText::new("Atalho").color(palette.soft).size(13.0).family(theme::semibold()));
            let (rect, _) = ui.allocate_exact_size(Vec2::new(KEYS_WIDTH, 40.0), Sense::hover());
            field = rect;
            let live = recording.is_some();
            let pulse = if live {
                ui.ctx().request_repaint();
                ((ui.input(|input| input.time) * 3.0).sin() * 0.5 + 0.5) as f32
            } else {
                0.0
            };
            let fill = if live { crate::motion::mix(field_fill(palette), palette.danger, 0.10 + 0.06 * pulse) } else { field_fill(palette) };
            ui.painter().rect_filled(rect, round, fill);
            let edge = if live { Stroke::new(1.5, palette.danger) } else { Stroke::new(1.0, palette.line_soft) };
            ui.painter().rect_stroke(rect, round, edge, egui::StrokeKind::Inside);

            let label = match (recording, shortcut.keys.is_empty()) {
                (Some(_), _) => "Parar gravação",
                (None, true) => "Gravar atalho",
                (None, false) => "Editar atalho",
            };
            let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::new(13.0, theme::semibold()), Color32::WHITE);
            let button = Rect::from_min_size(egui::pos2(rect.right() - 6.0 - galley.size().x - 24.0, rect.top() + 6.0), Vec2::new(galley.size().x + 24.0, rect.height() - 12.0));
            let response = ui.interact(button, ui.scope_id().with(("record", index)), Sense::click());
            let lit = crate::motion::hover(ui.ctx(), response.id, response.hovered());
            let (rest, over, ink) = if live { (palette.danger, palette.danger_hover, Color32::WHITE) } else { (palette.selected, palette.muted.gamma_multiply(0.55), palette.strong) };
            ui.painter().rect_filled(button, CornerRadius::same(if palette.glass { 8 } else { 4 }), crate::motion::mix(rest, over, lit));
            ui.painter().galley_with_override_text_color(button.center() - galley.size() / 2.0, galley, ink);
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                answer = RowAnswer::Record(!live);
            }

            let room = button.left() - rect.left() - 20.0;
            let at = rect.left_center() + Vec2::new(12.0, 0.0);
            match recording {
                Some([]) => widgets::paint_text_clipped(ui, at, room, "Pressione as teclas…", FontId::proportional(14.0), palette.danger_text),
                Some(keys) => widgets::paint_text_clipped(ui, at, room, &crate::shortcuts::describe(keys), FontId::new(14.0, theme::bold()), palette.strong),
                None if shortcut.keys.is_empty() => widgets::paint_text_clipped(ui, at, room, "Nenhum atalho definido", FontId::proportional(14.0), palette.muted),
                None => widgets::paint_text_clipped(ui, at, room, &crate::shortcuts::describe(&shortcut.keys), FontId::new(14.0, theme::bold()), palette.strong),
            }
        });
    });
    // The switch at the right edge, level with the fields.
    let switch_slot = Rect::from_center_size(egui::pos2(left + WIDTH - 20.0, field.center().y), Vec2::new(40.0, 24.0));
    let response = ui.interact(switch_slot, ui.scope_id().with(("switch", index)), Sense::click());
    paint_switch(ui, palette, response.id, switch_slot, shortcut.enabled, response.hovered());
    let tip = if shortcut.enabled { "Desativar atalho" } else { "Ativar atalho" };
    if response.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        answer = RowAnswer::Toggle;
    }
    ui.add_space(20.0);
    let row = Rect::from_min_max(egui::pos2(left, top), egui::pos2(left + WIDTH, ui.cursor().top()));
    // The cross that removes the row, at the right of its titles while the mouse is over it.
    let near = ui.rect_contains_pointer(row);
    let shown = crate::motion::ease(ui.ctx(), ui.scope_id().with(("remove-shown", index)), near, crate::motion::STATE);
    if shown > 0.0 {
        let spot = Rect::from_center_size(egui::pos2(row.right() - 20.0, field.top() - 18.0), Vec2::splat(22.0));
        let response = ui.interact(spot, ui.scope_id().with(("remove", index)), Sense::click());
        let fill = if response.hovered() { palette.danger } else { palette.selected };
        ui.painter().circle_filled(spot.center(), 11.0, fill.gamma_multiply(shown));
        let ink = if response.hovered() { Color32::WHITE } else { palette.soft };
        crate::icons::CLOSE.image(12.0, ink.gamma_multiply(shown)).paint_at(ui, Rect::from_center_size(spot.center(), Vec2::splat(12.0)));
        if response.on_hover_text("Remover atalho").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            answer = RowAnswer::Remove;
        }
    }
    let (line, _) = ui.allocate_exact_size(Vec2::new(WIDTH, 1.0), Sense::hover());
    ui.painter().rect_filled(line, CornerRadius::ZERO, palette.line_soft);
    answer
}

/// The fill of a field of the shortcuts.
fn field_fill(palette: &Palette) -> Color32 {
    if palette.glass { Color32::from_white_alpha(12) } else { palette.rail }
}

/// "+ Adicionar atalho", a quiet button with the plus at its left. True when clicked.
fn add_button(ui: &mut Ui, palette: &Palette) -> bool {
    let galley = ui.painter().layout_no_wrap("Adicionar atalho".to_owned(), FontId::new(14.0, theme::semibold()), palette.strong);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + 52.0, 38.0), Sense::click());
    let lit = crate::motion::hover(ui.ctx(), response.id, response.hovered());
    let round = CornerRadius::same(if palette.glass { 10 } else { 6 });
    ui.painter().rect_filled(rect, round, crate::motion::mix(palette.selected, palette.muted.gamma_multiply(0.55), lit));
    ui.painter().rect_stroke(rect, round, Stroke::new(1.0, palette.line_soft), egui::StrokeKind::Inside);
    crate::icons::PLUS.image(16.0, palette.strong).paint_at(ui, Rect::from_center_size(rect.left_center() + Vec2::new(22.0, 0.0), Vec2::splat(16.0)));
    ui.painter().galley(rect.left_center() + Vec2::new(36.0, -galley.size().y / 2.0), galley, palette.strong);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A box with a round "i" and one line, edged in its color, as Discord's notices in the settings.
fn info_banner(ui: &mut Ui, palette: &Palette, text: &str, tone: Color32) {
    let galley = ui.painter().layout(text.to_owned(), FontId::new(13.5, theme::semibold()), palette.strong, WIDTH - 60.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(WIDTH, galley.size().y + 24.0), Sense::hover());
    let round = CornerRadius::same(if palette.glass { 10 } else { 6 });
    ui.painter().rect_filled(rect, round, tone.gamma_multiply(0.12));
    ui.painter().rect_stroke(rect, round, Stroke::new(1.0, tone.gamma_multiply(0.8)), egui::StrokeKind::Inside);
    let dot = egui::pos2(rect.left() + 22.0, rect.center().y);
    ui.painter().circle_filled(dot, 8.0, tone);
    ui.painter().text(dot + Vec2::new(0.0, 0.5), egui::Align2::CENTER_CENTER, "i", FontId::new(11.0, theme::bold()), Color32::WHITE);
    ui.painter().galley(egui::pos2(rect.left() + 42.0, rect.center().y - galley.size().y / 2.0), galley, palette.strong);
}

/// The title of a part of the settings, as Discord sets them apart: in sentence case and larger,
/// and every part after the first opens with a thin line and room on both sides of it.
fn section(ui: &mut Ui, palette: &Palette, title: &str, first: bool) -> egui::Response {
    section_with(ui, palette, title, first, None).0
}

/// A [`section`] with a small button at the right of its title. True when it was clicked.
fn section_with(ui: &mut Ui, palette: &Palette, title: &str, first: bool, button: Option<&str>) -> (egui::Response, bool) {
    if first {
        ui.add_space(8.0);
    } else {
        ui.add_space(28.0);
        let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width().min(WIDTH), 1.0), Sense::hover());
        ui.painter().rect_filled(line, CornerRadius::ZERO, palette.line_soft);
        ui.add_space(28.0);
    }
    let title = sentence_case(title);
    let mut clicked = false;
    let response = ui
        .horizontal(|ui| {
            ui.set_width(ui.available_width().min(WIDTH));
            let label = ui.label(RichText::new(&title).color(palette.strong).size(17.0).family(theme::semibold()));
            if let Some(button) = button {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| clicked = small_button(ui, palette, button));
            }
            label
        })
        .inner;
    ui.add_space(14.0);
    (response, clicked)
}

/// The name of a field of the profile, as Discord's editor: no rule between them, room above.
fn field_title(ui: &mut Ui, palette: &Palette, title: &str, first: bool) -> egui::Response {
    ui.add_space(if first { 8.0 } else { 30.0 });
    let response = ui.label(RichText::new(title).color(palette.strong).size(15.0).family(theme::semibold()));
    ui.add_space(10.0);
    response
}

/// "TEMAS COLORIDOS" as "Temas coloridos".
fn sentence_case(title: &str) -> String {
    let lower = title.to_lowercase();
    let mut chars = lower.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// A setting with a slider in one line: the name in a column of its own, the slider and the value
/// at the right, so a group of them lines up as a table. For a group of knobs in a card.
fn slider_row(ui: &mut Ui, palette: &Palette, label: &str, shown: String, value: &mut f32, range: std::ops::RangeInclusive<f32>, step: f64) -> egui::Response {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(Vec2::new(width, 36.0), Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (name_rect, name) = ui.allocate_exact_size(Vec2::new(170.0, 36.0), Sense::hover());
        name.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label));
        ui.painter().text(name_rect.left_center(), egui::Align2::LEFT_CENTER, label, FontId::proportional(14.0), palette.text);
        let slider_width = width - 170.0 - 64.0;
        let response = ui
            .allocate_ui_with_layout(Vec2::new(slider_width, 36.0), Layout::left_to_right(Align::Center), |ui| {
                slider_look(ui, palette);
                ui.spacing_mut().slider_width = slider_width;
                ui.add(egui::Slider::new(value, range).step_by(step).show_value(false).trailing_fill(true)).labelled_by(name.id)
            })
            .inner;
        let (value_rect, _) = ui.allocate_exact_size(Vec2::new(64.0, 36.0), Sense::hover());
        ui.painter().text(value_rect.right_center(), egui::Align2::RIGHT_CENTER, shown, FontId::proportional(13.0), palette.muted);
        response
    })
    .inner
}

/// The colors of every slider of the settings: the accent up to the knob, a quiet rail after it.
fn slider_look(ui: &mut Ui, palette: &Palette) {
    ui.spacing_mut().slider_rail_height = 6.0;
    ui.style_mut().visuals.handle_shape = egui::style::HandleShape::Circle;
    let visuals = ui.visuals_mut();
    visuals.selection.bg_fill = palette.accent;
    visuals.widgets.inactive.bg_fill = if palette.dark { Color32::from_white_alpha(46) } else { Color32::from_gray(190) };
    for state in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active] {
        state.fg_stroke = Stroke::new(2.0, palette.accent);
    }
}

/// A card that holds a group of settings, with room inside, as Discord groups its rows.
fn card<R>(ui: &mut Ui, palette: &Palette, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let fill = if palette.glass { Color32::from_white_alpha(10) } else { palette.sidebar };
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, palette.line_soft))
        .corner_radius(CornerRadius::same(if palette.glass { 12 } else { 8 }))
        .inner_margin(Margin { left: 18, right: 18, top: 10, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(WIDTH - 36.0);
            contents(ui)
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_cut_in_the_middle_to_the_wanted_shape() {
        let wide = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(900, 300, |x, _| if x < 300 || x >= 600 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) }));
        let mut bytes = Vec::new();
        wide.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        let avatar = prepare_picture(&bytes, (256, 256)).unwrap();
        assert!(avatar.len() <= MAX_AVATAR_BYTES);
        let decoded = image::load_from_memory(&avatar).unwrap();
        assert_eq!(decoded.dimensions(), (256, 256));
        // Only the blue middle third is left.
        let pixel = decoded.get_pixel(128, 128);
        assert!(pixel[2] > 200 && pixel[0] < 60);
        let corner = decoded.get_pixel(4, 128);
        assert!(corner[2] > 200 && corner[0] < 60);
        assert!(prepare_picture(b"not a picture", (16, 16)).is_err());
    }

    #[test]
    fn a_surprise_color_is_dark_on_the_dark_base_and_light_on_the_light_one() {
        for _ in 0..40 {
            let dark = random_color(false);
            let light = random_color(true);
            let brightness = |color: Color32| u32::from(color.r()) + u32::from(color.g()) + u32::from(color.b());
            assert!(brightness(dark) < 520, "{dark:?}");
            assert!(brightness(light) > 430, "{light:?}");
        }
    }
}
