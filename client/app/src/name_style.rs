//! The style each person gave their profile: the effect on the name, the colors of the card and
//! the "about me". Everyone's style arrives with their presence and is kept, so a name stays
//! styled in the history while the person is offline.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, RichText, Sense, Ui, Vec2};
use telinha_core::profile::{Style, clean_style};

use crate::app::TelinhaApp;
use crate::theme::{self, Palette};
use crate::widgets;

const MAX_REMEMBERED: usize = 300;
/// How long the colors of a moving name take to go around once, in seconds.
const FLOW_SECONDS: f64 = 3.0;

pub struct Styles {
    file: PathBuf,
    known: BTreeMap<String, Style>,
}

impl Styles {
    pub fn load(data_dir: &Path) -> Self {
        let file = data_dir.join("styles.json");
        let stored: BTreeMap<String, serde_json::Value> = std::fs::read_to_string(&file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        // What is on disk is checked like what arrives from other people.
        let known = stored.iter().map(|(member, style)| (member.clone(), clean_style(Some(style)))).filter(|(_, style)| style.is_set()).take(MAX_REMEMBERED).collect();
        Self { file, known }
    }

    /// Keeps the style of someone. True when it changed.
    pub fn remember(&mut self, member_id: &str, style: Option<&Style>) -> bool {
        if member_id.is_empty() {
            return false;
        }
        let kept = style.filter(|style| style.is_set());
        if self.known.get(member_id) == kept {
            return false;
        }
        match kept {
            Some(style) => {
                if self.known.len() >= MAX_REMEMBERED && !self.known.contains_key(member_id) {
                    self.known.pop_first();
                }
                self.known.insert(member_id.to_owned(), style.clone());
            }
            None => {
                self.known.remove(member_id);
            }
        }
        if let Ok(text) = serde_json::to_string(&self.known) {
            // Without the disk the styles hold for this run.
            let _ = std::fs::write(&self.file, text);
        }
        true
    }

    pub fn of(&self, member_id: Option<&str>) -> Option<&Style> {
        self.known.get(member_id?)
    }
}

pub fn parse_color(hex: &str) -> Option<Color32> {
    let digits = hex.strip_prefix('#').filter(|digits| digits.len() == 6)?;
    let part = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some(Color32::from_rgb(part(0)?, part(2)?, part(4)?))
}

pub fn write_color(color: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
}

fn mix(from: Color32, to: Color32, amount: f32) -> Color32 {
    let part = |from: u8, to: u8| (f32::from(from) + (f32::from(to) - f32::from(from)) * amount).round() as u8;
    Color32::from_rgb(part(from.r(), to.r()), part(from.g(), to.g()), part(from.b(), to.b()))
}

/// The color at a point of a row of colors, where 0 is the first and 1 the last.
fn along(colors: &[Color32], point: f32) -> Color32 {
    let scaled = point.clamp(0.0, 1.0) * (colors.len() - 1) as f32;
    let index = (scaled.floor() as usize).min(colors.len() - 2);
    mix(colors[index], colors[index + 1], scaled - index as f32)
}

/// The colors of a name and whether it glows. None is a name without an effect.
fn effect_of(style: Option<&Style>) -> Option<(Vec<Color32>, bool)> {
    let name = style?.name.as_ref()?;
    let colors: Vec<Color32> = name.colors.iter().filter_map(|color| parse_color(color)).collect();
    match (name.effect.as_str(), colors.len()) {
        ("solid", 1..) => Some((vec![colors[0]], false)),
        ("neon", 1..) => Some((vec![colors[0]], true)),
        ("gradient" | "prism", 2..) => Some((colors, false)),
        _ => None,
    }
}

/// A name ready to draw, each letter in its color. `phase` moves the colors along the name.
pub fn name_job(text: &str, font: FontId, base: Color32, style: Option<&Style>, phase: f32) -> (LayoutJob, Option<Color32>) {
    let mut job = LayoutJob::default();
    let format = |color: Color32| TextFormat { font_id: font.clone(), color, ..Default::default() };
    match effect_of(style) {
        None => job.append(text, 0.0, format(base)),
        Some((colors, glow)) if colors.len() == 1 => {
            job.append(text, 0.0, format(colors[0]));
            return (job, glow.then_some(colors[0]));
        }
        Some((mut colors, _)) => {
            // The first color comes back at the end, so the colors join when they run. The name
            // shows half of the row at a time, as the app has always drawn it.
            colors.push(colors[0]);
            let count = text.chars().count().max(1) as f32;
            let mut letter = [0u8; 4];
            for (index, character) in text.chars().enumerate() {
                let point = (index as f32 / count * 0.5 + phase).fract();
                job.append(character.encode_utf8(&mut letter), 0.0, format(along(&colors, point)));
            }
        }
    }
    (job, None)
}

fn phase_of(ui: &Ui, moving: bool) -> f32 {
    if !moving {
        return 0.0;
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(40));
    (ui.input(|input| input.time) / FLOW_SECONDS).fract() as f32
}

fn paint_galley(ui: &Ui, at: Pos2, galley: std::sync::Arc<egui::Galley>, glow: Option<Color32>, clip: Option<Rect>) {
    let painter = match clip {
        Some(clip) => ui.painter().with_clip_rect(clip),
        None => ui.painter().clone(),
    };
    if let Some(glow) = glow {
        // A soft light around the letters: the name drawn again and again, faint, a little off.
        let faint = glow.gamma_multiply(0.16);
        for step in 0..8 {
            let angle = std::f32::consts::TAU * step as f32 / 8.0;
            painter.galley_with_override_text_color(at + Vec2::angled(angle) * 1.6, std::sync::Arc::clone(&galley), faint);
        }
    }
    painter.galley(at, galley, Color32::WHITE);
}

/// Paints a name on one line, cut where the room ends. Returns the width it took.
pub fn paint_name(ui: &Ui, left_center: Pos2, width: f32, text: &str, font: FontId, base: Color32, style: Option<&Style>) -> f32 {
    let (job, glow) = name_job(text, font, base, style, 0.0);
    let galley = ui.painter().layout_job(job);
    let at = left_center - Vec2::new(0.0, galley.size().y / 2.0);
    let used = galley.size().x.min(width);
    // The light of a glowing name may pass the edge a little; the letters may not.
    let clip = Rect::from_min_max(at - Vec2::new(4.0, 6.0), egui::pos2(at.x + width, at.y + galley.size().y + 6.0));
    paint_galley(ui, at, galley, glow, Some(clip));
    used
}

/// A name as a piece of the layout that can be clicked. The colors run while the mouse is over
/// it, or always when `moving` says so.
pub fn name_label(ui: &mut Ui, text: &str, font: FontId, base: Color32, style: Option<&Style>, moving: bool) -> Response {
    let (plain, _) = name_job(text, font.clone(), base, style, 0.0);
    let size = ui.painter().layout_job(plain).size();
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let flows = effect_of(style).is_some_and(|(colors, _)| colors.len() > 1);
    let phase = phase_of(ui, flows && (moving || response.hovered()));
    let (job, glow) = name_job(text, font, base, style, phase);
    let galley = ui.painter().layout_job(job);
    paint_galley(ui, rect.min, galley, glow, None);
    response
}

/// Who a profile card is about, and where on the window it was asked for.
#[derive(Debug, Clone)]
pub struct CardRequest {
    pub member_id: Option<String>,
    pub name: String,
    pub anchor: Rect,
}

pub const CARD_WIDTH: f32 = 300.0;

/// What a profile card shows.
pub struct CardFace<'a> {
    pub name: &'a str,
    pub activity: &'a str,
    pub style: Option<&'a Style>,
    pub avatar: Option<egui::Image<'static>>,
    pub banner: Option<egui::Image<'static>>,
    pub online: bool,
    /// Adds the button that opens the settings, for the card of the person themself.
    pub edit: bool,
    pub shadow: bool,
}

/// A rectangle with round corners filled with a color that changes from point to point.
fn rounded_fill(rect: Rect, corners: [f32; 4], color_at: impl Fn(Pos2) -> Color32) -> egui::Shape {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), color_at(rect.center()));
    // Around the outline, corner by corner: top left, top right, bottom right, bottom left.
    let places = [
        (rect.left_top(), Vec2::new(1.0, 1.0), 180.0f32, corners[0]),
        (rect.right_top(), Vec2::new(-1.0, 1.0), 270.0, corners[1]),
        (rect.right_bottom(), Vec2::new(-1.0, -1.0), 0.0, corners[2]),
        (rect.left_bottom(), Vec2::new(1.0, -1.0), 90.0, corners[3]),
    ];
    const STEPS: u32 = 8;
    for (corner, inward, from, radius) in places {
        let center = corner + inward * radius;
        for step in 0..=STEPS {
            let angle = (from + 90.0 * step as f32 / STEPS as f32).to_radians();
            let point = center + Vec2::angled(angle) * radius;
            mesh.colored_vertex(point, color_at(point));
        }
    }
    let count = 4 * (STEPS + 1);
    for index in 0..count {
        mesh.add_triangle(0, 1 + index, 1 + (index + 1) % count);
    }
    egui::Shape::mesh(mesh)
}

/// Where the parts of a card drawn by [`card_spots`] are, and whether its edit button was clicked.
pub struct CardSpots {
    pub edit: bool,
    pub banner: Rect,
    pub portrait: Rect,
}

/// Draws a profile card at the cursor, 300 wide. True when its edit button was clicked.
pub fn card_ui(ui: &mut Ui, palette: &Palette, face: CardFace<'_>) -> bool {
    card_spots(ui, palette, face).edit
}

/// Draws a profile card at the cursor, 300 wide, and tells where its banner and its picture are.
pub fn card_spots(ui: &mut Ui, palette: &Palette, face: CardFace<'_>) -> CardSpots {
    use crate::appearance::Gradient;
    let theme_colors = face.style.and_then(|style| style.theme.as_ref()).and_then(|colors| Some((parse_color(colors.first()?)?, parse_color(colors.get(1)?)?)));
    // With a theme, the two colors are the frame and the middle is darkened so text reads over
    // any color.
    let flow = theme_colors.map(|(first, second)| Gradient { light: false, colors: vec![first, second], angle: 165.0, intensity: 100.0, motion: false });
    let dark = Color32::from_rgb(8, 8, 11);
    let (strong, soft, text, line) = if flow.is_some() {
        (Color32::WHITE, Color32::from_white_alpha(184), Color32::from_rgb(0xe6, 0xe7, 0xea), Color32::from_white_alpha(41))
    } else {
        (palette.strong, palette.muted, palette.text, palette.line_soft)
    };
    let mut edit = false;
    let outer_top = ui.cursor().min;
    let back = ui.painter().add(egui::Shape::Noop);
    let inner_left = outer_top.x + 3.0;
    let inner_width = CARD_WIDTH - 6.0;
    let banner_height = if face.banner.is_some() { inner_width * 210.0 / 600.0 } else { 60.0 };
    let strip = Rect::from_min_size(egui::pos2(inner_left, outer_top.y + 3.0), Vec2::new(inner_width, banner_height));
    let banner_shape = ui.painter().add(egui::Shape::Noop);
    let has_banner = face.banner.is_some();
    if let Some(banner) = face.banner {
        banner.corner_radius(CornerRadius { nw: 11, ne: 11, sw: 0, se: 0 }).paint_at(ui, strip);
    }
    let ring_shape = ui.painter().add(egui::Shape::Noop);
    let portrait = Rect::from_center_size(egui::pos2(inner_left + 58.0, strip.bottom() - 2.0), Vec2::splat(76.0));
    widgets::paint_avatar(ui, portrait, face.name, face.avatar);
    let dot_shape = ui.painter().add(egui::Shape::Noop);

    let body = Rect::from_min_size(egui::pos2(inner_left + 16.0, strip.bottom() + 48.0), Vec2::new(inner_width - 32.0, 0.0));
    let mut content = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(body.min, Vec2::new(body.width(), f32::INFINITY))).layout(egui::Layout::top_down(egui::Align::Min)));
    content.spacing_mut().item_spacing.y = 2.0;
    name_label(&mut content, face.name, FontId::new(20.0, theme::bold()), strong, face.style, true);
    if let Some(pronouns) = face.style.map(|style| style.pronouns.as_str()).filter(|pronouns| !pronouns.is_empty()) {
        content.label(RichText::new(pronouns).color(text).size(13.0));
    }
    content.label(RichText::new(face.activity).color(soft).size(13.0));
    if let Some(bio) = face.style.map(|style| style.bio.as_str()).filter(|bio| !bio.is_empty()) {
        content.add_space(12.0);
        let (rule, _) = content.allocate_exact_size(Vec2::new(body.width(), 1.0), Sense::hover());
        content.painter().rect_filled(rule, CornerRadius::ZERO, line);
        content.add_space(11.0);
        content.label(RichText::new("Sobre mim").color(strong).size(12.0).family(theme::bold()));
        content.add_space(2.0);
        content.label(RichText::new(bio).color(text).size(14.0));
    }
    if face.edit {
        content.add_space(14.0);
        let (rect, response) = content.allocate_exact_size(Vec2::new(body.width(), 36.0), Sense::click());
        let fill = match (theme_colors, response.hovered()) {
            (Some((first, _)), false) => mix(first, Color32::BLACK, 0.22),
            (Some((first, _)), true) => first,
            (None, false) => palette.accent,
            (None, true) => palette.accent_hover,
        };
        content.painter().rect_filled(rect, CornerRadius::same(4), fill);
        content.painter().text(rect.center(), Align2::CENTER_CENTER, "Editar perfil", FontId::new(14.0, theme::semibold()), Color32::WHITE);
        edit = response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    }
    let bottom = content.min_rect().bottom() + 16.0;
    let inner = Rect::from_min_max(egui::pos2(inner_left, outer_top.y + 3.0), egui::pos2(inner_left + inner_width, bottom));
    let outer = inner.expand(3.0);
    ui.allocate_rect(outer, Sense::hover());

    // Now that the height is known, what goes under everything is painted in the places kept.
    let surface_at = |point: Pos2| match &flow {
        Some(flow) => mix(flow.color_at(outer, point, 0.0), dark, 0.66),
        None => palette.float,
    };
    let mut shapes = Vec::new();
    if face.shadow {
        let glow = theme_colors.map_or(Color32::from_black_alpha(128), |(first, _)| first.gamma_multiply(0.45));
        shapes.push(egui::epaint::Shadow { offset: [0, 8], blur: 26, spread: 0, color: glow }.as_shape(outer, CornerRadius::same(14)).into());
    }
    shapes.push(rounded_fill(outer, [14.0; 4], |point| flow.as_ref().map_or(palette.line_soft, |flow| flow.color_at(outer, point, 0.0))));
    shapes.push(rounded_fill(inner, [11.0; 4], surface_at));
    ui.painter().set(back, egui::Shape::Vec(shapes));
    if !has_banner {
        let fill = rounded_fill(strip, [11.0, 11.0, 0.0, 0.0], |point| flow.as_ref().map_or(palette.accent, |flow| flow.color_at(outer, point, 0.0)));
        ui.painter().set(banner_shape, fill);
    }
    let ring = surface_at(portrait.center());
    ui.painter().set(ring_shape, egui::Shape::circle_filled(portrait.center(), 44.0, ring));
    let dot_center = portrait.right_bottom() - Vec2::splat(6.0);
    let dot = vec![
        egui::Shape::circle_filled(dot_center, 10.0, ring),
        egui::Shape::circle_filled(dot_center, 6.0, if face.online { palette.green } else { Color32::from_rgb(0x80, 0x84, 0x8e) }),
    ];
    ui.painter().set(dot_shape, egui::Shape::Vec(dot));
    CardSpots { edit, banner: strip, portrait }
}
impl TelinhaApp {
    pub fn open_card(&mut self, member_id: Option<String>, name: &str, anchor: Rect) {
        self.card = Some(CardRequest { member_id, name: name.to_owned(), anchor });
        self.card_opened = self.context.input(|input| input.time);
    }

    pub fn close_card(&mut self) {
        self.card = None;
    }

    /// What someone is doing now, looking at every place where they show up online.
    pub(crate) fn activity_of(&self, member_id: Option<&str>) -> String {
        let place = |space_id: &str, room_id: &str, live: bool| -> String {
            let Some(server) = self.servers.iter().find(|server| server.id == space_id) else {
                return if live { "Transmitindo numa chamada" } else { "Em chamada" }.to_owned();
            };
            let room = server.channels.iter().find(|channel| channel.id == room_id).map_or("uma sala", |channel| channel.name.as_str());
            if live { format!("Transmitindo em {room}") } else { format!("Na sala {room}") }
        };
        let Some(member_id) = member_id else { return "Offline".to_owned() };
        if member_id == self.hub.member_id() {
            let live = self.room.streams.outgoing.is_some();
            return self.room.voice.as_ref().map_or_else(|| "Online".to_owned(), |voice| place(&voice.space_id, &voice.room_id, live));
        }
        let mut online = false;
        let spaces = self.servers.iter().map(|server| server.id.clone()).chain(self.contacts.iter().map(|(contact, _)| contact.id.clone()));
        for space_id in spaces {
            for peer in self.hub.peers(&space_id).iter().filter(|peer| peer.member_id.as_deref() == Some(member_id)) {
                online = true;
                if let Some(voice) = &peer.voice {
                    return place(&space_id, &voice.room_id, voice.live);
                }
            }
        }
        if online { "Online" } else { "Offline" }.to_owned()
    }

    /// The card of a person, next to where it was asked for, until a click somewhere else.
    pub(crate) fn show_card(&mut self, context: &egui::Context) {
        let Some(request) = self.card.clone() else { return };
        let palette = self.palette;
        let style = self.styles.of(request.member_id.as_deref()).cloned();
        let activity = self.activity_of(request.member_id.as_deref());
        let me = request.member_id.as_deref() == Some(self.hub.member_id().as_str());
        let avatar = self.pictures.avatar(&self.hub, request.member_id.as_deref());
        let banner = self.pictures.banner(&self.hub, request.member_id.as_deref());
        // Whoever is on the right half of the window, like the list of members, gets the card on
        // their left.
        let screen = context.content_rect();
        let on_right = request.anchor.center().x > screen.center().x;
        let wanted = if on_right { request.anchor.left() - CARD_WIDTH - 12.0 } else { request.anchor.right() + 12.0 };
        let left = wanted.clamp(screen.left() + 8.0, (screen.right() - CARD_WIDTH - 8.0).max(screen.left() + 8.0));
        let top = (request.anchor.top() - 24.0).clamp(screen.top() + 8.0, (screen.bottom() - 330.0).max(screen.top() + 8.0));
        let mut edit = false;
        let area = egui::Area::new(egui::Id::unique("profile-card")).order(egui::Order::Foreground).fixed_pos(egui::pos2(left, top)).show(context, |ui| {
            let face = CardFace { name: &request.name, activity: &activity, style: style.as_ref(), avatar, banner, online: activity != "Offline", edit: me, shadow: true };
            edit = card_ui(ui, &palette, face);
        });
        // The click that opened the card is also a click outside of it.
        let fresh = context.input(|input| input.time) - self.card_opened < 0.2;
        let escaped = context.input(|input| input.key_pressed(egui::Key::Escape));
        if edit {
            self.card = None;
            self.open_settings();
        } else if escaped || (!fresh && area.response.clicked_elsewhere()) {
            self.card = None;
        }
    }
}

/// The name of someone in a row of the list of members, in their style, and under it what they
/// are doing, when they are doing something.
#[allow(clippy::too_many_arguments)]
pub fn paint_row_name(ui: &Ui, rect: Rect, left: f32, text: &str, palette: &Palette, lit: bool, style: Option<&Style>, activity: Option<&str>) {
    let base = if lit { palette.text } else { palette.muted };
    let lift = if activity.is_some() { 8.0 } else { 0.0 };
    let left_center = rect.left_center() + Vec2::new(left, -lift);
    paint_name(ui, left_center, rect.width() - left - 8.0, text, FontId::new(15.0, theme::semibold()), base, style);
    if let Some(activity) = activity {
        widgets::paint_text_clipped(ui, rect.left_center() + Vec2::new(left, 9.0), rect.width() - left - 8.0, activity, FontId::proportional(12.0), palette.muted);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn style(value: serde_json::Value) -> Style {
        clean_style(Some(&value))
    }

    #[test]
    fn styles_are_kept_by_person_and_dropped_when_emptied() {
        let folder = tempfile::tempdir().unwrap();
        let mut styles = Styles::load(folder.path());
        let neon = style(json!({ "name": { "effect": "neon", "colors": ["#ff0080"] }, "bio": "oi" }));
        assert!(styles.remember("a", Some(&neon)));
        assert!(!styles.remember("a", Some(&neon)), "the same style is not a change");
        assert!(!styles.remember("", Some(&neon)));
        assert!(!styles.remember("b", Some(&Style::default())), "an empty style is nothing to keep");
        let again = Styles::load(folder.path());
        assert_eq!(again.of(Some("a")), Some(&neon));
        assert_eq!(again.of(Some("b")), None);
        assert_eq!(again.of(None), None);
        assert!(styles.remember("a", None));
        assert_eq!(Styles::load(folder.path()).of(Some("a")), None);
    }

    #[test]
    fn each_effect_colors_the_letters_its_way() {
        let font = FontId::proportional(14.0);
        let colors = |job: &LayoutJob| job.sections.iter().map(|section| section.format.color).collect::<Vec<_>>();
        let (plain, glow) = name_job("Keep", font.clone(), Color32::GRAY, None, 0.0);
        assert_eq!((colors(&plain), glow), (vec![Color32::GRAY], None));

        let neon = style(json!({ "name": { "effect": "neon", "colors": ["#ff0080"] } }));
        let (job, glow) = name_job("Keep", font.clone(), Color32::GRAY, Some(&neon), 0.0);
        assert_eq!((colors(&job), glow), (vec![Color32::from_rgb(255, 0, 128)], Some(Color32::from_rgb(255, 0, 128))));

        // Two colors: the name at rest goes from the first towards the second.
        let gradient = style(json!({ "name": { "effect": "gradient", "colors": ["#000000", "#ffffff"] } }));
        let (job, glow) = name_job("abcd", font.clone(), Color32::GRAY, Some(&gradient), 0.0);
        let shades: Vec<u8> = colors(&job).iter().map(|color| color.r()).collect();
        assert_eq!((shades.len(), shades[0], glow), (4, 0, None));
        assert!(shades.windows(2).all(|pair| pair[0] < pair[1]), "{shades:?}");
        // Half a turn later it starts from the second color.
        let (moved, _) = name_job("abcd", font, Color32::GRAY, Some(&gradient), 0.5);
        assert_eq!(colors(&moved)[0], Color32::WHITE);

        assert_eq!(parse_color("#19b8b0"), Some(Color32::from_rgb(0x19, 0xb8, 0xb0)));
        assert_eq!(parse_color("19b8b0"), None);
        assert_eq!(write_color(Color32::from_rgb(0x19, 0xb8, 0xb0)), "#19b8b0");
    }
}
