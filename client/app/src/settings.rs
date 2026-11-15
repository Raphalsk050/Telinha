//! The settings window: who the person is, the voice devices and how the app looks. The same
//! sections in the same order as the settings of the old app.

use egui::{Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};
use image::GenericImageView;
use telinha_core::avatars::MAX_AVATAR_BYTES;
use telinha_core::profile::{NameStyle, Style};

use crate::appearance::{Appearance, COLOR_THEMES, CustomTheme, Density, Gradient, MAX_CUSTOM_COLORS, MessageStyle, ThemeChoice};
use crate::dialogs::{Tone, button, text_field};
use crate::name_style::{CARD_WIDTH, CardFace, card_ui, parse_color, write_color};
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
    /// The camera to use, by name. None is the first one found.
    Camera(Option<String>),
    Blur(bool),
    /// Start or stop the test of the camera.
    Preview(bool),
    ShowLog,
    Notice(String),
}

pub struct Settings {
    name: String,
    /// The microphones and the speakers plugged in when the window opened.
    inputs: Vec<String>,
    outputs: Vec<String>,
    cameras: Vec<String>,
    /// The style of the profile as the person is changing it, and when it gets saved.
    style: Style,
    style_due: Option<f64>,
    /// The size while its slider is being dragged. It only takes hold when let go.
    zoom_drag: Option<f32>,
    /// How far down the window is scrolled, when a test wants a part further down.
    scroll_to: Option<f32>,
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
    pub camera: Option<&'a str>,
    pub blur: bool,
    /// Whether the camera is being tested, and its last frame.
    pub previewing: bool,
    pub preview: Option<egui::load::SizedTexture>,
    pub camera_status: &'a str,
}

const EFFECTS: [(&str, &str, usize, usize); 5] = [("", "Nenhum", 0, 0), ("solid", "Sólido", 1, 1), ("gradient", "Degradê", 2, 2), ("neon", "Neon", 1, 1), ("prism", "Prisma", 2, 5)];
const DEFAULT_COLORS: [&str; 5] = ["#5865f2", "#eb459e", "#19b8b0", "#f0b232", "#23a55a"];
/// How long after the last change the style is saved and told to everyone.
const STYLE_DELAY: f64 = 0.4;
const WIDTH: f32 = 680.0;

/// One choice of a row of choices, lit when it is the one in force.
fn chip(ui: &mut Ui, palette: &Palette, label: &str, chosen: bool) -> bool {
    chip_button(ui, palette, label, chosen).clicked()
}

/// The same button, for who wants more than the click.
pub(crate) fn chip_button(ui: &mut Ui, palette: &Palette, label: &str, chosen: bool) -> egui::Response {
    let ink = if chosen { Color32::WHITE } else { palette.text };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::new(14.0, theme::semibold()), ink);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + 24.0, 32.0), Sense::click());
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
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(15.0), palette.text);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(28.0 + galley.size().x, 26.0), Sense::click());
    let square = Rect::from_center_size(egui::pos2(rect.left() + 9.0, rect.center().y), Vec2::splat(18.0));
    if on {
        ui.painter().rect_filled(square, CornerRadius::same(4), palette.accent);
        let tick = [square.left_center() + Vec2::new(4.0, 0.5), square.center() + Vec2::new(-1.5, 4.0), square.right_top() + Vec2::new(-4.0, 5.0)];
        ui.painter().add(egui::Shape::line(tick.to_vec(), Stroke::new(2.0, Color32::WHITE)));
    } else {
        ui.painter().rect_stroke(square, CornerRadius::same(4), Stroke::new(1.5, palette.muted), egui::StrokeKind::Inside);
    }
    ui.painter().galley(egui::pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0), galley, palette.text);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
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
    let width = galley.size().x.max(52.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 80.0), Sense::click());
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
    ui.add_space(6.0);
    let mut name = None;
    ui.horizontal(|ui| {
        name = Some(ui.label(RichText::new(label).color(palette.text).size(14.0)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(shown).color(palette.muted).size(13.0));
        });
    });
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = ui.available_width();
        ui.spacing_mut().slider_rail_height = 6.0;
        ui.style_mut().visuals.handle_shape = egui::style::HandleShape::Circle;
        let visuals = ui.visuals_mut();
        visuals.selection.bg_fill = palette.accent;
        visuals.widgets.inactive.bg_fill = Color32::from_gray(if palette.dark { 232 } else { 190 });
        for state in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active] {
            state.fg_stroke = Stroke::new(2.0, palette.accent);
        }
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
        Self { name: name.to_owned(), inputs, outputs, cameras, style, style_due: None, zoom_drag: None, scroll_to: None }
    }

    /// Shows the window scrolled this far down. For the tests.
    pub fn scroll_for_test(&mut self, offset: f32) {
        self.scroll_to = Some(offset);
    }

    /// The effect of the name, the colors of the card, the banner and the "about me", with the
    /// card as others see it on the right. True when the style changed.
    fn style_editor(&mut self, ui: &mut Ui, palette: &Palette, view: &mut SettingsView<'_>, actions: &mut Vec<SettingsAction>) -> bool {
        let mut changed = false;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.vertical(|ui| {
                ui.set_width(WIDTH - CARD_WIDTH - 16.0);
                ui.spacing_mut().item_spacing.y = 10.0;
                let current = self.style.name.as_ref().map_or("", |name| name.effect.as_str()).to_owned();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (effect, label, least, most) in EFFECTS {
                        if chip(ui, palette, label, current == effect) && current != effect {
                            changed = true;
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
                    let (least, most) = EFFECTS.iter().find(|(effect, ..)| *effect == name.effect).map_or((1, 1), |(_, _, least, most)| (*least, *most));
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let removable = name.colors.len() > least;
                        let mut remove = None;
                        for (index, text) in name.colors.iter_mut().enumerate() {
                            let mut color = parse_color(text).unwrap_or(Color32::GRAY);
                            let (edited, removed) = swatch(ui, palette, &mut color, removable);
                            if edited {
                                *text = write_color(color);
                                changed = true;
                            }
                            if removed {
                                remove = Some(index);
                            }
                        }
                        if let Some(index) = remove {
                            name.colors.remove(index);
                            changed = true;
                        }
                        if name.colors.len() < most && small_button(ui, palette, "Adicionar cor") {
                            name.colors.push(DEFAULT_COLORS[name.colors.len() % DEFAULT_COLORS.len()].to_owned());
                            changed = true;
                        }
                    });
                }
                let themed = self.style.theme.is_some();
                if check(ui, palette, themed, "Colorir meu cartão de perfil") {
                    self.style.theme = (!themed).then(|| DEFAULT_COLORS[..2].iter().map(|color| (*color).to_owned()).collect());
                    changed = true;
                }
                if let Some(colors) = &mut self.style.theme {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for text in colors {
                            let mut color = parse_color(text).unwrap_or(Color32::GRAY);
                            if swatch(ui, palette, &mut color, false).0 {
                                *text = write_color(color);
                                changed = true;
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if small_button(ui, palette, "Mudar banner") {
                        pick_picture(true, actions);
                    }
                    if view.banner.is_some() && crate::debug_view::ghost_button(ui, palette, "Remover banner").clicked() {
                        actions.push(SettingsAction::RemovePicture { banner: true });
                    }
                });
                let about = Frame::new().fill(palette.rail).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.style.bio)
                            .frame(Frame::new())
                            .font(FontId::proportional(15.0))
                            .text_color(palette.text)
                            .hint_text(RichText::new("Sobre mim").color(palette.muted))
                            .char_limit(190)
                            .desired_rows(3)
                            .desired_width(ui.available_width()),
                    )
                });
                changed |= about.inner.changed();
                ui.label(RichText::new("Quem estiver em uma conversa com você vê seu nome e seu cartão assim.").color(palette.muted).size(13.0));
            });
            ui.vertical(|ui| {
                ui.set_width(CARD_WIDTH);
                let face = CardFace { name: view.name, activity: view.activity, style: Some(&self.style), avatar: view.avatar.clone(), banner: view.banner.clone(), online: true, edit: false, shadow: false };
                card_ui(ui, palette, face);
            });
        });
        changed
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
    pub fn show(&mut self, context: &egui::Context, palette: &Palette, mut view: SettingsView<'_>) -> (Vec<SettingsAction>, bool) {
        let mut actions = Vec::new();
        let mut open = true;
        let mut look = view.appearance.clone();
        let frame = Frame::new().fill(palette.main).corner_radius(CornerRadius::same(8)).stroke(Stroke::new(1.0, palette.line)).inner_margin(Margin::same(20));
        let response = egui::Modal::new(egui::Id::unique("settings")).frame(frame).backdrop_color(Color32::from_black_alpha(178)).show(context, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Configurações").color(palette.strong).size(18.0).family(theme::bold()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, crate::icons::CLOSE, 30.0, palette.soft, palette.hover, "Fechar").clicked() {
                        open = false;
                    }
                });
            });

            // In a short window the settings scroll instead of running off the screen.
            let room = (context.content_rect().height() - 110.0).max(200.0);
            let mut area = egui::ScrollArea::vertical().max_height(room).min_scrolled_height(room).auto_shrink([false, true]);
            if let Some(offset) = self.scroll_to {
                area = area.vertical_scroll_offset(offset);
            }
            area.show(ui, |ui| {
                ui.set_width(WIDTH);

                section(ui, palette, "AVATAR", true);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let has_avatar = view.avatar.is_some();
                    widgets::avatar(ui, 72.0, view.name, view.avatar.clone());
                    if button(ui, palette, "Mudar avatar", Tone::Primary).clicked() {
                        actions.push(SettingsAction::AvatarPicker);
                    }
                    if has_avatar && button(ui, palette, "Remover avatar", Tone::Ghost).clicked() {
                        actions.push(SettingsAction::RemovePicture { banner: false });
                    }
                });

                let label = section(ui, palette, "SEU NOME", false);
                ui.horizontal(|ui| {
                    ui.allocate_ui(Vec2::new(WIDTH - 82.0, 38.0), |ui| text_field(ui, palette, &mut self.name).labelled_by(label.id));
                    if button(ui, palette, "Salvar", Tone::Primary).clicked() && !self.name.trim().is_empty() && self.name.trim() != view.name {
                        actions.push(SettingsAction::Name(self.name.trim().to_owned()));
                    }
                });

                section(ui, palette, "ESTILO DO NOME", false);
                if self.style_editor(ui, palette, &mut view, &mut actions) {
                    let now = ui.input(|input| input.time);
                    self.style_due = Some(now + STYLE_DELAY);
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(STYLE_DELAY));
                }

                section(ui, palette, "VOZ", false);
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
                // As `.meter`: the green bar of how loud the microphone is.
                ui.add_space(2.0);
                let (bar, _) = ui.allocate_exact_size(Vec2::new(WIDTH, 8.0), Sense::hover());
                ui.painter().rect_filled(bar, CornerRadius::same(4), palette.rail);
                let level = view.mic_level.clamp(0.0, 1.0);
                if level > 0.0 {
                    let fill = Rect::from_min_size(bar.min, Vec2::new(bar.width() * level, bar.height()));
                    ui.painter().with_clip_rect(fill).rect_filled(bar, CornerRadius::same(4), palette.green);
                }
                if !view.mic_status.is_empty() {
                    ui.label(RichText::new(view.mic_status).color(palette.muted).size(13.0));
                }

                section(ui, palette, "VÍDEO", false);
                self.video_section(ui, palette, &view, &mut actions);

                section(ui, palette, "TEMA", false);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    for (choice, label, theme) in [
                        (ThemeChoice::Light, "Claro", Some(ThemeName::Light)),
                        (ThemeChoice::Ash, "Cinza", Some(ThemeName::Gray)),
                        (ThemeChoice::Dark, "Escuro", Some(ThemeName::Dark)),
                        (ThemeChoice::Onyx, "Ônix", Some(ThemeName::Onyx)),
                        (ThemeChoice::System, "Igual ao computador", None),
                    ] {
                        let clicked = theme_dot(ui, palette, label, look.theme == choice, |painter, center, radius| match theme {
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

                section(ui, palette, "TEMAS COLORIDOS", false);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(16.0, 8.0);
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

                section(ui, palette, "DENSIDADE DA INTERFACE", false);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (density, _, label) in Density::ALL {
                        if chip(ui, palette, label, look.density == density) {
                            look.density = density;
                        }
                    }
                });

                section(ui, palette, "MENSAGENS", false);
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

                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if crate::debug_view::ghost_button(ui, palette, "Abrir registro de erros").on_hover_text("Abre a pasta do arquivo que registra erros do app").clicked() {
                        actions.push(SettingsAction::ShowLog);
                    }
                    ui.label(RichText::new(format!("Telinha {}", view.version)).color(palette.muted).size(13.0));
                });
            });
        });
        if response.should_close() {
            open = false;
        }
        if look != *view.appearance {
            actions.push(SettingsAction::Appearance(Box::new(look)));
        }
        // The style is saved a moment after the last change, or at once when the window closes.
        let now = context.input(|input| input.time);
        if self.style_due.is_some_and(|due| now >= due || !open) {
            self.style_due = None;
            if let Ok(style) = serde_json::to_value(&self.style) {
                actions.push(SettingsAction::Style(style));
            }
        }
        (actions, open)
    }
}

impl Settings {
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

/// The title of a part of the settings. The first one has less room above it.
fn section(ui: &mut Ui, palette: &Palette, title: &str, first: bool) -> egui::Response {
    ui.add_space(if first { 4.0 } else { 14.0 });
    let response = ui.label(RichText::new(title).color(palette.soft).size(12.0).family(theme::bold()));
    ui.add_space(2.0);
    response
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
