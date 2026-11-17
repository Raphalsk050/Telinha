//! How the app looks: the theme, the color themes with a gradient behind everything, how tight
//! the rows are, how messages are drawn and the sizes. The same choices, with the same names and
//! limits, as `desktop/renderer/appearance.js`.

use egui::{Color32, Pos2, Rect, Vec2};
use serde_json::{Value, json};

use crate::name_style::{parse_color, write_color};
use crate::theme::{Palette, ThemeName};

/// A theme made of a gradient behind the whole app, over the dark or the light base.
#[derive(Debug, PartialEq)]
pub struct ColorTheme {
    pub id: &'static str,
    pub name: &'static str,
    pub light: bool,
    pub angle: f32,
    pub colors: &'static [u32],
}

pub const COLOR_THEMES: [ColorTheme; 14] = [
    ColorTheme { id: "aurora", name: "Aurora", light: false, angle: 135.0, colors: &[0x0b3d4f, 0x1f8a70, 0x7b2ff7] },
    ColorTheme { id: "sunset", name: "Pôr do sol", light: false, angle: 160.0, colors: &[0x48217a, 0xc2416b, 0xf08a4b] },
    ColorTheme { id: "ocean", name: "Oceano", light: false, angle: 150.0, colors: &[0x0a2a66, 0x0f6fa8, 0x19b8b0] },
    ColorTheme { id: "forest", name: "Floresta", light: false, angle: 140.0, colors: &[0x0f3d2e, 0x2f7d4f, 0xa6b85a] },
    ColorTheme { id: "midnight", name: "Meia-noite", light: false, angle: 135.0, colors: &[0x1a1446, 0x3b3fb5, 0x5865f2] },
    ColorTheme { id: "crimson", name: "Lua carmesim", light: false, angle: 150.0, colors: &[0x3a0d18, 0x8a1c2b, 0xd1495b] },
    ColorTheme { id: "neon", name: "Neon", light: false, angle: 120.0, colors: &[0x3d0f6b, 0xc2187a, 0x12b3d6] },
    ColorTheme { id: "dusk", name: "Crepúsculo", light: false, angle: 170.0, colors: &[0x2b2f77, 0x6b4fa0, 0xc47fb0] },
    ColorTheme { id: "mint", name: "Menta", light: true, angle: 135.0, colors: &[0xbff0d4, 0xe6f7b2] },
    ColorTheme { id: "citrus", name: "Cítrico", light: true, angle: 135.0, colors: &[0xffd9a0, 0xff9aa2] },
    ColorTheme { id: "candy", name: "Algodão-doce", light: true, angle: 135.0, colors: &[0xf9c2e3, 0xb8d8ff] },
    ColorTheme { id: "blossom", name: "Cerejeira", light: true, angle: 150.0, colors: &[0xffd1dc, 0xe8c7f5, 0xc9e4ff] },
    ColorTheme { id: "sunrise", name: "Nascer do sol", light: true, angle: 160.0, colors: &[0xffe29a, 0xffa99f, 0xc5b3ff] },
    ColorTheme { id: "sepia", name: "Sépia", light: true, angle: 135.0, colors: &[0xe8d8bf, 0xcbb393] },
];
const PRESET_INTENSITY: f32 = 75.0;
pub const MAX_CUSTOM_COLORS: usize = 5;

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    Light,
    Ash,
    Dark,
    Onyx,
    /// Light or gray, as Windows is set.
    System,
    /// The gradient the person made.
    Custom,
    /// One of the color themes, by its place in the list.
    Color(usize),
    /// Translucent panels that blur a dark wallpaper with colored lights.
    Glass,
}

impl ThemeChoice {
    pub fn key(self) -> String {
        match self {
            Self::Light => "light".to_owned(),
            Self::Ash => "ash".to_owned(),
            Self::Dark => "dark".to_owned(),
            Self::Onyx => "onyx".to_owned(),
            Self::System => "system".to_owned(),
            Self::Custom => "custom".to_owned(),
            Self::Color(index) => format!("color:{}", COLOR_THEMES[index].id),
            Self::Glass => "glass".to_owned(),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "light" => Some(Self::Light),
            // The first versions of this app called the gray theme by another name.
            "ash" | "gray" => Some(Self::Ash),
            "dark" => Some(Self::Dark),
            "onyx" => Some(Self::Onyx),
            "system" => Some(Self::System),
            "custom" => Some(Self::Custom),
            "glass" => Some(Self::Glass),
            other => COLOR_THEMES.iter().position(|theme| other.strip_prefix("color:") == Some(theme.id)).map(Self::Color),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Density {
    Compact,
    Default,
    Spacious,
}

impl Density {
    pub const ALL: [(Self, &'static str, &'static str); 3] = [(Self::Compact, "compact", "Compacta"), (Self::Default, "default", "Padrão"), (Self::Spacious, "spacious", "Espaçosa")];

    /// The room above and below the text of a row of the sidebar.
    pub fn row_pad(self) -> f32 {
        [3.0, 6.0, 9.0][self as usize]
    }

    pub fn member_pad(self) -> f32 {
        [2.0, 5.0, 8.0][self as usize]
    }

    pub fn header_height(self) -> f32 {
        [40.0, 48.0, 56.0][self as usize]
    }

    /// The space between the buttons of the servers.
    pub fn rail_gap(self) -> f32 {
        [4.0, 8.0, 12.0][self as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStyle {
    /// With the picture of who wrote.
    Cozy,
    /// Without it, to fit more conversation on the screen.
    Compact,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CustomTheme {
    pub light: bool,
    pub colors: Vec<Color32>,
    pub angle: f32,
    pub intensity: f32,
}

impl Default for CustomTheme {
    fn default() -> Self {
        Self { light: false, colors: vec![rgb(0x5865f2), rgb(0xeb459e)], angle: 135.0, intensity: 50.0 }
    }
}

/// The five knobs of the glass theme, with the limits and the defaults of the sketch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassLook {
    /// The white haze and the grain on the glass, 0 to 1.
    pub frost: f32,
    /// How dark the glass is, 0 to 1.
    pub dark: f32,
    /// The blur, in points, 0 to 80.
    pub blur: f32,
    /// How much the lights of the wallpaper show, 0 to 1.6; above 1 they brighten.
    pub light: f32,
    /// How large the lights are, 0.3 to 2.5.
    pub light_size: f32,
}

impl Default for GlassLook {
    fn default() -> Self {
        Self { frost: 0.0, dark: 0.66, blur: 16.0, light: 0.02, light_size: 1.05 }
    }
}

impl GlassLook {
    /// Each knob: its key in the file, its name, its limits and its step.
    pub const KNOBS: [(&'static str, &'static str, f32, f32, f64); 5] = [
        ("frost", "Jateamento", 0.0, 1.0, 0.01),
        ("dark", "Escurecimento", 0.0, 1.0, 0.01),
        ("blur", "Desfoque", 0.0, 80.0, 1.0),
        ("light", "Luz de fundo", 0.0, 1.6, 0.01),
        ("lightSize", "Tamanho das luzes", 0.3, 2.5, 0.05),
    ];

    pub fn knob(&mut self, index: usize) -> &mut f32 {
        match index {
            0 => &mut self.frost,
            1 => &mut self.dark,
            2 => &mut self.blur,
            3 => &mut self.light,
            _ => &mut self.light_size,
        }
    }

    /// The tint of the main panel: `rgba(16, 18, 34, dark)`.
    pub fn main(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(16, 18, 34, (self.dark.clamp(0.0, 1.0) * 255.0).round() as u8)
    }

    /// The tint of the sidebar, a little darker: `rgba(16, 18, 34, min(1, dark + .1))`.
    pub fn side(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(16, 18, 34, ((self.dark + 0.1).clamp(0.0, 1.0) * 255.0).round() as u8)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Appearance {
    pub theme: ThemeChoice,
    /// Keeps the sidebar dark in the light theme.
    pub dark_sidebar: bool,
    /// Lets the gradient of a color theme drift slowly.
    pub motion: bool,
    pub density: Density,
    pub messages: MessageStyle,
    /// The size of the text of the messages, from 12 to 24.
    pub chat_font: f32,
    /// The space between blocks of messages, from 0 to 24.
    pub message_gap: f32,
    /// The size of everything, in percent, from 50 to 200.
    pub zoom: f32,
    pub custom: CustomTheme,
    pub glass: GlassLook,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            // Glass unless the person picks another one.
            theme: ThemeChoice::Glass,
            dark_sidebar: false,
            motion: false,
            density: Density::Default,
            messages: MessageStyle::Cozy,
            chat_font: 15.5,
            message_gap: 16.0,
            zoom: 100.0,
            custom: CustomTheme::default(),
            glass: GlassLook::default(),
        }
    }
}

/// The gradient of a color theme: what is painted behind the app.
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub light: bool,
    pub colors: Vec<Color32>,
    pub angle: f32,
    pub intensity: f32,
    pub motion: bool,
}

/// The fill of each region of the window. With a gradient behind, each is a veil over it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Regions {
    pub main: Color32,
    pub side: Color32,
    pub rail: Color32,
    /// The strip of the person and of the call at the foot of the sidebar.
    pub foot: Color32,
}

fn mix(tint: Color32, share: f32, base: u32) -> Color32 {
    let base = rgb(base);
    let part = |tint: u8, base: u8| (f32::from(tint) * share + f32::from(base) * (1.0 - share)).round() as u8;
    Color32::from_rgb(part(tint.r(), base.r()), part(tint.g(), base.g()), part(tint.b(), base.b()))
}

fn luminance(color: Color32) -> f32 {
    let channel = |value: u8| {
        let unit = f32::from(value) / 255.0;
        if unit <= 0.03928 { unit / 12.92 } else { ((unit + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

impl Gradient {
    /// How opaque the veil over the gradient is: thinner the stronger the theme, down to where
    /// text still reads over the strongest color of the gradient.
    pub fn veil_alpha(&self) -> f32 {
        let lights = self.colors.iter().map(|color| luminance(*color));
        let harshest = if self.light { 1.0 - lights.fold(f32::MAX, f32::min) } else { lights.fold(0.0, f32::max) };
        let readable = 0.5 + 0.38 * harshest;
        (0.9 - 0.4 * (self.intensity / 100.0)).max(readable)
    }

    fn tint(&self) -> Color32 {
        let count = self.colors.len().max(1) as f32;
        let average = |pick: fn(&Color32) -> u8| (self.colors.iter().map(|color| f32::from(pick(color))).sum::<f32>() / count).round() as u8;
        Color32::from_rgb(average(Color32::r), average(Color32::g), average(Color32::b))
    }

    /// The color of the gradient at a point of a rectangle, as CSS draws a linear gradient at
    /// this angle: 0 goes up, 90 goes right.
    pub fn color_at(&self, rect: Rect, point: Pos2, shift: f32) -> Color32 {
        let angle = self.angle.to_radians();
        let direction = Vec2::new(angle.sin(), -angle.cos());
        let length = (rect.width() * direction.x).abs() + (rect.height() * direction.y).abs();
        let along = if length > 0.0 { (point - rect.center()).dot(direction) / length + 0.5 } else { 0.5 };
        let place = (along + shift).clamp(0.0, 1.0);
        if self.colors.len() < 2 {
            return self.colors.first().copied().unwrap_or(Color32::BLACK);
        }
        let scaled = place * (self.colors.len() - 1) as f32;
        let index = (scaled.floor() as usize).min(self.colors.len() - 2);
        let (from, to, amount) = (self.colors[index], self.colors[index + 1], scaled - index as f32);
        let part = |from: u8, to: u8| (f32::from(from) + (f32::from(to) - f32::from(from)) * amount).round() as u8;
        Color32::from_rgb(part(from.r(), to.r()), part(from.g(), to.g()), part(from.b(), to.b()))
    }

    /// Paints the gradient over a rectangle, as a grid fine enough for the bends between colors.
    pub fn paint(&self, painter: &egui::Painter, rect: Rect, time: f64) {
        // In motion the gradient is drawn over an area 2.2 times the window and slides across
        // it and back, every 24 seconds.
        let (area, shift) = if self.motion {
            let phase = ((time / 24.0) % 2.0) as f32;
            let slide = if phase > 1.0 { 2.0 - phase } else { phase };
            let eased = slide * slide * (3.0 - 2.0 * slide);
            let extra = rect.size() * 1.2;
            (Rect::from_min_size(rect.min - extra * eased, rect.size() * 2.2), 0.0)
        } else {
            (rect, 0.0)
        };
        const CELLS: usize = 16;
        let mut mesh = egui::Mesh::default();
        for row in 0..=CELLS {
            for column in 0..=CELLS {
                let point = rect.min + Vec2::new(rect.width() * column as f32 / CELLS as f32, rect.height() * row as f32 / CELLS as f32);
                mesh.colored_vertex(point, self.color_at(area, point, shift));
            }
        }
        let side = (CELLS + 1) as u32;
        for row in 0..CELLS as u32 {
            for column in 0..CELLS as u32 {
                let corner = row * side + column;
                mesh.add_triangle(corner, corner + 1, corner + side);
                mesh.add_triangle(corner + 1, corner + side, corner + side + 1);
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }

    /// A circle filled with the gradient, for the button that picks the theme.
    pub fn paint_dot(&self, painter: &egui::Painter, center: Pos2, radius: f32) {
        let rect = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(center, self.color_at(rect, center, 0.0));
        const STEPS: u32 = 40;
        for step in 0..=STEPS {
            let point = center + Vec2::angled(std::f32::consts::TAU * step as f32 / STEPS as f32) * radius;
            mesh.colored_vertex(point, self.color_at(rect, point, 0.0));
        }
        for step in 1..=STEPS {
            mesh.add_triangle(0, step, step + 1);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

impl Appearance {
    /// Reads the choices as the old app wrote them, keeping only what is within the limits.
    pub fn from_json(source: Option<&Value>) -> Self {
        let mut look = Self::default();
        let Some(source) = source.filter(|source| source.is_object()) else { return look };
        let text = |name: &str| source.get(name).and_then(Value::as_str);
        let number = |name: &str, low: f32, high: f32| source.get(name).and_then(Value::as_f64).filter(|value| value.is_finite()).map(|value| (value as f32).clamp(low, high));
        if let Some(theme) = text("theme").and_then(ThemeChoice::from_key) {
            look.theme = theme;
        }
        if let Some((density, ..)) = Density::ALL.iter().find(|(_, key, _)| text("density") == Some(key)) {
            look.density = *density;
        }
        look.messages = if text("messages") == Some("compact") { MessageStyle::Compact } else { MessageStyle::Cozy };
        look.chat_font = number("chatFont", 12.0, 24.0).unwrap_or(look.chat_font);
        look.message_gap = number("messageGap", 0.0, 24.0).unwrap_or(look.message_gap);
        look.zoom = number("zoom", 50.0, 200.0).unwrap_or(look.zoom);
        look.dark_sidebar = source.get("darkSidebar").and_then(Value::as_bool) == Some(true);
        look.motion = source.get("motion").and_then(Value::as_bool) == Some(true);
        if let Some(custom) = source.get("custom").filter(|custom| custom.is_object()) {
            let colors: Vec<Color32> = custom.get("colors").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).filter_map(parse_color).take(MAX_CUSTOM_COLORS).collect();
            let number = |name: &str, low: f32, high: f32| custom.get(name).and_then(Value::as_f64).filter(|value| value.is_finite()).map(|value| (value as f32).clamp(low, high));
            look.custom = CustomTheme {
                light: custom.get("base").and_then(Value::as_str) == Some("light"),
                colors: if colors.is_empty() { CustomTheme::default().colors } else { colors },
                angle: number("angle", 0.0, 360.0).unwrap_or(135.0),
                intensity: number("intensity", 0.0, 100.0).unwrap_or(50.0),
            };
        }
        if let Some(glass) = source.get("glass").filter(|glass| glass.is_object()) {
            for (index, (key, _, low, high, _)) in GlassLook::KNOBS.iter().enumerate() {
                if let Some(value) = glass.get(*key).and_then(Value::as_f64).filter(|value| value.is_finite()) {
                    *look.glass.knob(index) = (value as f32).clamp(*low, *high);
                }
            }
        }
        look
    }

    pub fn to_json(&self) -> Value {
        json!({
            "theme": self.theme.key(),
            "darkSidebar": self.dark_sidebar,
            "motion": self.motion,
            "density": Density::ALL[self.density as usize].1,
            "messages": if self.messages == MessageStyle::Compact { "compact" } else { "cozy" },
            "chatFont": self.chat_font,
            "messageGap": self.message_gap,
            "zoom": self.zoom,
            "custom": {
                "base": if self.custom.light { "light" } else { "dark" },
                "colors": self.custom.colors.iter().map(|color| write_color(*color)).collect::<Vec<_>>(),
                "angle": self.custom.angle,
                "intensity": self.custom.intensity,
            },
            "glass": {
                "frost": self.glass.frost,
                "dark": self.glass.dark,
                "blur": self.glass.blur,
                "light": self.glass.light,
                "lightSize": self.glass.light_size,
            },
        })
    }

    /// The gradient behind the app, when the theme is one of the color themes.
    pub fn gradient(&self) -> Option<Gradient> {
        match self.theme {
            ThemeChoice::Custom => Some(Gradient { light: self.custom.light, colors: self.custom.colors.clone(), angle: self.custom.angle, intensity: self.custom.intensity, motion: self.motion }),
            ThemeChoice::Color(index) => {
                let theme = &COLOR_THEMES[index];
                Some(Gradient { light: theme.light, colors: theme.colors.iter().map(|color| rgb(*color)).collect(), angle: theme.angle, intensity: PRESET_INTENSITY, motion: self.motion })
            }
            _ => None,
        }
    }

    /// The plain theme under everything: the one chosen, the one Windows uses, or the base of
    /// the color theme.
    pub fn base(&self, system_light: bool) -> ThemeName {
        match (self.gradient(), self.theme) {
            (None, ThemeChoice::Glass) => ThemeName::Dark,
            (Some(gradient), _) => if gradient.light { ThemeName::Light } else { ThemeName::Dark },
            (None, ThemeChoice::Light) => ThemeName::Light,
            (None, ThemeChoice::Dark) => ThemeName::Dark,
            (None, ThemeChoice::Onyx) => ThemeName::Onyx,
            (None, ThemeChoice::System) if system_light => ThemeName::Light,
            (None, _) => ThemeName::Gray,
        }
    }

    /// The colors of the window and the fill of each of its regions.
    pub fn palette(&self, system_light: bool) -> (Palette, Regions) {
        if self.theme == ThemeChoice::Glass {
            let glass = &self.glass;
            return (Palette::glass(), Regions { main: glass.main(), side: glass.side(), rail: Color32::TRANSPARENT, foot: Color32::from_black_alpha(46) });
        }
        let mut palette = Palette::of(self.base(system_light));
        let Some(gradient) = self.gradient() else {
            return (palette, Regions { main: palette.main, side: palette.sidebar, rail: palette.rail, foot: palette.panel });
        };
        let tint = gradient.tint();
        let alpha = gradient.veil_alpha();
        let (veil, hover, selected, line, soft) = if gradient.light {
            ((255, 255, 255), Color32::from_black_alpha(15), Color32::from_black_alpha(26), Color32::from_black_alpha(31), Color32::from_black_alpha(46))
        } else {
            ((12, 12, 14), Color32::from_white_alpha(20), Color32::from_white_alpha(36), Color32::from_black_alpha(89), Color32::from_white_alpha(36))
        };
        // What stays on top, like windows, cards and fields, is still opaque, only pulled
        // towards the color of the theme.
        if gradient.light {
            palette.rail = mix(tint, 0.16, 0xe3e5e8);
            palette.sidebar = mix(tint, 0.12, 0xf2f3f5);
            palette.main = mix(tint, 0.08, 0xffffff);
            palette.panel = mix(tint, 0.14, 0xebedef);
            palette.raised = mix(tint, 0.16, 0xe3e5e8);
            palette.float = mix(tint, 0.05, 0xffffff);
        } else {
            palette.rail = mix(tint, 0.10, 0x121214);
            palette.sidebar = mix(tint, 0.12, 0x17171a);
            palette.main = mix(tint, 0.14, 0x1c1c20);
            palette.panel = mix(tint, 0.10, 0x131316);
            palette.raised = mix(tint, 0.14, 0x27272c);
            palette.float = mix(tint, 0.08, 0x0a0a0c);
        }
        (palette.hover, palette.selected, palette.line, palette.line_soft) = (hover, selected, line, soft);
        let veil_of = |alpha: f32| Color32::from_rgba_unmultiplied(veil.0, veil.1, veil.2, (alpha * 255.0).round() as u8);
        let rail = veil_of((alpha + 0.14).min(0.96));
        (palette, Regions { main: veil_of(alpha), side: veil_of((alpha + 0.07).min(0.94)), rail, foot: rail })
    }

    /// The colors of the rail and of the sidebar, which may stay dark in the light theme.
    /// The knobs of the glass theme, when it is the one on.
    pub fn glass_look(&self) -> Option<GlassLook> {
        (self.theme == ThemeChoice::Glass).then_some(self.glass)
    }

    pub fn side_palette(&self, system_light: bool) -> Option<Palette> {
        let light = self.gradient().is_none() && self.base(system_light) == ThemeName::Light;
        (light && self.dark_sidebar).then_some(Palette::GRAY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choices_are_read_and_written_as_the_old_app_did() {
        let stored = json!({
            "theme": "color:ocean", "darkSidebar": true, "motion": true, "density": "spacious", "messages": "compact",
            "chatFont": 99, "messageGap": 8, "zoom": 10,
            "custom": { "base": "light", "colors": ["#FF0000", "nope", "#00ff00"], "angle": 400, "intensity": 30 },
        });
        let look = Appearance::from_json(Some(&stored));
        assert_eq!(look.theme, ThemeChoice::Color(2));
        assert_eq!((look.density, look.messages, look.dark_sidebar, look.motion), (Density::Spacious, MessageStyle::Compact, true, true));
        assert_eq!((look.chat_font, look.message_gap, look.zoom), (24.0, 8.0, 50.0));
        assert_eq!(look.custom, CustomTheme { light: true, colors: vec![Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 255, 0)], angle: 360.0, intensity: 30.0 });
        assert_eq!(Appearance::from_json(Some(&look.to_json())), look);

        // Nonsense, or nothing at all, gives the defaults. The old name of the gray theme works.
        assert_eq!(Appearance::from_json(None), Appearance::default());
        assert_eq!(Appearance::from_json(Some(&json!({ "theme": "plaid", "density": 3 }))), Appearance::default());
        assert_eq!(Appearance::from_json(Some(&json!({ "theme": "gray" }))).theme, ThemeChoice::Ash);
        assert_eq!(Appearance::from_json(None).theme, ThemeChoice::Glass);
        let glass = Appearance::from_json(Some(&json!({ "theme": "glass", "glass": { "frost": 0.4, "dark": 3, "blur": 12, "lightSize": "big" } })));
        assert_eq!(glass.theme, ThemeChoice::Glass);
        assert_eq!(glass.glass, GlassLook { frost: 0.4, dark: 1.0, blur: 12.0, ..GlassLook::default() });
        assert_eq!(Appearance::from_json(Some(&glass.to_json())), glass);
        for choice in [ThemeChoice::Light, ThemeChoice::System, ThemeChoice::Custom, ThemeChoice::Color(13), ThemeChoice::Glass] {
            assert_eq!(ThemeChoice::from_key(&choice.key()), Some(choice));
        }
    }

    #[test]
    fn a_plain_theme_fills_the_regions_and_a_color_theme_veils_them() {
        let plain = Appearance { theme: ThemeChoice::Ash, ..Appearance::default() };
        let (palette, regions) = plain.palette(false);
        assert_eq!(palette, Palette::GRAY);
        assert_eq!((regions.main, regions.side, regions.rail), (palette.main, palette.sidebar, palette.rail));
        assert_eq!(Appearance { theme: ThemeChoice::System, ..plain.clone() }.base(true), ThemeName::Light);
        assert_eq!(Appearance { theme: ThemeChoice::System, ..plain.clone() }.base(false), ThemeName::Gray);

        // Aurora: the veil is the dark base, see-through, more so in the middle than at the rail.
        let aurora = Appearance { theme: ThemeChoice::Color(0), ..plain.clone() };
        let gradient = aurora.gradient().unwrap();
        assert!((gradient.veil_alpha() - 0.6).abs() < 0.03, "alpha {}", gradient.veil_alpha());
        let (palette, regions) = aurora.palette(false);
        assert!(palette.dark && regions.main.a() < regions.side.a() && regions.side.a() < regions.rail.a() && regions.rail.a() < 255);
        assert_ne!(palette.main, Palette::of(ThemeName::Dark).main, "the surfaces lean to the theme");
        // A light color theme sits on the light base.
        assert!(!Appearance { theme: ThemeChoice::Color(8), ..plain.clone() }.palette(false).0.dark);

        // The sidebar stays dark only in the light theme, and only when asked.
        assert!(plain.side_palette(false).is_none());
        let light = Appearance { theme: ThemeChoice::Light, dark_sidebar: true, ..plain };
        assert_eq!(light.side_palette(false), Some(Palette::GRAY));
        assert!(Appearance { theme: ThemeChoice::Color(8), ..light }.side_palette(false).is_none());
    }

    #[test]
    fn a_gradient_goes_from_its_first_color_to_its_last_along_its_angle() {
        let gradient = Gradient { light: false, colors: vec![Color32::BLACK, Color32::WHITE], angle: 90.0, intensity: 50.0, motion: false };
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 100.0));
        // 90 degrees goes to the right.
        assert_eq!(gradient.color_at(rect, egui::pos2(0.0, 50.0), 0.0), Color32::BLACK);
        assert_eq!(gradient.color_at(rect, egui::pos2(200.0, 50.0), 0.0), Color32::WHITE);
        assert_eq!(gradient.color_at(rect, egui::pos2(100.0, 0.0), 0.0).r(), 128);
        // 180 degrees goes down.
        let down = Gradient { angle: 180.0, ..gradient };
        assert_eq!(down.color_at(rect, egui::pos2(50.0, 0.0), 0.0), Color32::BLACK);
        assert_eq!(down.color_at(rect, egui::pos2(50.0, 100.0), 0.0), Color32::WHITE);
    }
}
