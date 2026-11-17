//! The colors and fonts of the app. The default theme is the gray one; the others only swap the
//! surface colors, as the old stylesheet did.

use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeName {
    Gray,
    Dark,
    Onyx,
    Light,
}

impl ThemeName {
    pub const ALL: [Self; 4] = [Self::Gray, Self::Dark, Self::Onyx, Self::Light];

    pub fn label(self) -> &'static str {
        match self {
            Self::Gray => "Cinza",
            Self::Dark => "Escuro",
            Self::Onyx => "Ônix",
            Self::Light => "Claro",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Gray => "gray",
            Self::Dark => "dark",
            Self::Onyx => "onyx",
            Self::Light => "light",
        }
    }

    pub fn from_key(key: &str) -> Self {
        Self::ALL.into_iter().find(|theme| theme.key() == key).unwrap_or(Self::Gray)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub rail: Color32,
    pub sidebar: Color32,
    pub main: Color32,
    pub panel: Color32,
    pub raised: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub line: Color32,
    pub line_soft: Color32,
    pub text: Color32,
    pub strong: Color32,
    pub muted: Color32,
    pub soft: Color32,
    pub green: Color32,
    pub float: Color32,
    pub link: Color32,
    pub danger_text: Color32,
    pub accent_text: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub yellow: Color32,
    pub danger: Color32,
    pub danger_hover: Color32,
    pub dark: bool,
    /// The glass theme: translucent surfaces, gradient buttons, thin scroll bars.
    pub glass: bool,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Palette {
    pub const GRAY: Self = Self {
        rail: rgb(0x1e1f22),
        sidebar: rgb(0x2b2d31),
        main: rgb(0x313338),
        panel: rgb(0x232428),
        raised: rgb(0x383a40),
        hover: rgb(0x35373c),
        selected: rgb(0x404249),
        line: rgb(0x1f2023),
        line_soft: rgb(0x3f4147),
        text: rgb(0xdbdee1),
        strong: rgb(0xf2f3f5),
        muted: rgb(0x949ba4),
        soft: rgb(0xb5bac1),
        green: rgb(0x23a55a),
        float: rgb(0x111214),
        link: rgb(0x00a8fc),
        danger_text: rgb(0xfa777c),
        accent_text: rgb(0x949cf7),
        accent: rgb(0x5865f2),
        accent_hover: rgb(0x4752c4),
        yellow: rgb(0xf0b232),
        danger: rgb(0xda373c),
        danger_hover: rgb(0xa12828),
        dark: true,
        glass: false,
    };

    /// The glass theme, with the colors of `[data-theme="glass"]` in the sketch. What lies on
    /// a surface is translucent white; what stands alone (fields, windows not made of glass,
    /// menus) stays opaque in the dark blue of the glass.
    pub fn glass() -> Self {
        let white = Color32::from_white_alpha;
        Self {
            rail: rgb(0x0e0f1d),
            sidebar: rgb(0x15172b),
            main: rgb(0x191b30),
            panel: white(13),
            raised: white(20),
            hover: white(18),
            selected: white(36),
            line: white(20),
            line_soft: white(36),
            text: rgb(0xe8eaf4),
            strong: rgb(0xffffff),
            muted: rgb(0xa9aecb),
            soft: rgb(0xc8cce0),
            float: Color32::from_rgba_unmultiplied(16, 18, 30, 240),
            accent: rgb(0x6d72ff),
            accent_hover: rgb(0x5a5ff0),
            glass: true,
            ..Self::GRAY
        }
    }

    pub fn of(theme: ThemeName) -> Self {
        match theme {
            ThemeName::Gray => Self::GRAY,
            ThemeName::Dark => Self {
                rail: rgb(0x121214),
                sidebar: rgb(0x17171a),
                main: rgb(0x1c1c20),
                panel: rgb(0x131316),
                raised: rgb(0x27272c),
                hover: rgb(0x24242a),
                selected: rgb(0x2e2e35),
                line: rgb(0x0c0c0e),
                line_soft: rgb(0x303037),
                float: rgb(0x0a0a0c),
                ..Self::GRAY
            },
            ThemeName::Onyx => Self {
                rail: rgb(0x000000),
                sidebar: rgb(0x060607),
                main: rgb(0x0b0b0d),
                panel: rgb(0x000000),
                raised: rgb(0x1a1a1e),
                hover: rgb(0x16161a),
                selected: rgb(0x202026),
                line: rgb(0x1c1c21),
                line_soft: rgb(0x2a2a31),
                float: rgb(0x050506),
                ..Self::GRAY
            },
            ThemeName::Light => Self {
                rail: rgb(0xe3e5e8),
                sidebar: rgb(0xf2f3f5),
                main: rgb(0xffffff),
                panel: rgb(0xebedef),
                raised: rgb(0xe3e5e8),
                hover: rgb(0xe6e8eb),
                selected: rgb(0xd7d9dc),
                line: rgb(0xe1e2e4),
                line_soft: rgb(0xc4c9ce),
                text: rgb(0x313338),
                strong: rgb(0x060607),
                muted: rgb(0x5c5e66),
                soft: rgb(0x4e5058),
                green: rgb(0x1f8b4c),
                float: rgb(0xffffff),
                link: rgb(0x006ce7),
                danger_text: rgb(0xd22d39),
                accent_text: rgb(0x4752c4),
                dark: false,
                ..Self::GRAY
            },
        }
    }
}

/// The colors initials are drawn on when someone has no picture. The same list and the same
/// rule as before, so everyone keeps their color.
const AVATAR_COLORS: [Color32; 8] = [
    rgb(0x5865f2),
    rgb(0x3ba55c),
    rgb(0xfaa61a),
    rgb(0xed4245),
    rgb(0xeb459e),
    rgb(0x9b59b6),
    rgb(0x1abc9c),
    rgb(0xe67e22),
];

pub fn avatar_color(seed: &str) -> Color32 {
    let mut hash: u32 = 0;
    for unit in seed.encode_utf16() {
        hash = hash.wrapping_mul(31).wrapping_add(u32::from(unit));
    }
    AVATAR_COLORS[(hash % AVATAR_COLORS.len() as u32) as usize]
}

pub fn initial(name: &str) -> String {
    name.chars().next().map_or_else(|| "?".to_owned(), |first| first.to_uppercase().collect())
}

pub const BOLD: &str = "bold";
pub const SEMIBOLD: &str = "semibold";

pub fn bold() -> FontFamily {
    FontFamily::Name(BOLD.into())
}

pub fn semibold() -> FontFamily {
    FontFamily::Name(SEMIBOLD.into())
}

/// Loads Segoe UI from Windows, the font the app has always used. Where it is missing, the
/// font that comes with egui stays.
pub fn install_fonts(context: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let folder = std::env::var_os("WINDIR").map_or_else(|| "C:\\Windows".into(), std::path::PathBuf::from).join("Fonts");
    let mut load = |name: &str, file: &str| {
        let Ok(bytes) = std::fs::read(folder.join(file)) else {
            return false;
        };
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
        true
    };
    let regular = load("segoe", "segoeui.ttf");
    let semi = load("segoe-semibold", "seguisb.ttf");
    let heavy = load("segoe-bold", "segoeuib.ttf");
    let symbols = load("segoe-symbol", "seguisym.ttf");

    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let family = |first: Option<&str>| {
        let mut list: Vec<String> = first.into_iter().map(str::to_owned).collect();
        if regular && first != Some("segoe") {
            list.push("segoe".to_owned());
        }
        list.extend(fallbacks.iter().cloned());
        if symbols {
            list.push("segoe-symbol".to_owned());
        }
        list
    };
    fonts.families.insert(FontFamily::Proportional, family(regular.then_some("segoe")));
    fonts.families.insert(semibold(), family(semi.then_some("segoe-semibold")));
    fonts.families.insert(bold(), family(heavy.then_some("segoe-bold")));
    context.set_fonts(fonts);
}

/// Makes the stock egui widgets match the palette: text fields, buttons, scroll bars, menus.
pub fn apply(context: &egui::Context, palette: &Palette) {
    let mut visuals = if palette.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.panel_fill = palette.main;
    visuals.window_fill = palette.main;
    visuals.extreme_bg_color = palette.rail;
    visuals.faint_bg_color = palette.panel;
    visuals.override_text_color = Some(palette.text);
    // The hint of an empty field: egui 0.37 always paints it in this color, and Chromium in #757575.
    visuals.weak_text_color = Some(egui::Color32::from_gray(117));
    visuals.hyperlink_color = palette.link;
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.55);
    visuals.selection.stroke = egui::Stroke::new(1.0, palette.strong);
    visuals.window_stroke = egui::Stroke::new(1.0, palette.line);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, palette.line_soft);
    visuals.widgets.inactive.bg_fill = palette.raised;
    visuals.widgets.inactive.weak_bg_fill = palette.raised;
    visuals.widgets.hovered.bg_fill = palette.selected;
    visuals.widgets.hovered.weak_bg_fill = palette.selected;
    visuals.widgets.active.bg_fill = palette.selected;
    visuals.widgets.active.weak_bg_fill = palette.selected;
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.bg_stroke = egui::Stroke::NONE;
        widget.corner_radius = egui::CornerRadius::same(4);
        widget.fg_stroke = egui::Stroke::new(1.0, palette.text);
    }
    if palette.glass {
        // The thumb of the thin scroll bars of the sketch: white at 16%, 32% under the mouse.
        visuals.widgets.inactive.bg_fill = Color32::from_white_alpha(41);
        visuals.widgets.hovered.bg_fill = Color32::from_white_alpha(82);
        visuals.widgets.active.bg_fill = Color32::from_white_alpha(82);
    }
    context.set_visuals(visuals);
    context.all_styles_mut(|style| {
        style.spacing.scroll = if palette.glass {
            // A 4 point thumb 3 points from the edge, 6 under the mouse, over the content.
            egui::style::ScrollStyle {
                floating: true,
                bar_width: 6.0,
                floating_width: 4.0,
                floating_allocated_width: 0.0,
                handle_min_length: 40.0,
                bar_inner_margin: 3.0,
                bar_outer_margin: 3.0,
                foreground_color: false,
                dormant_background_opacity: 0.0,
                active_background_opacity: 0.0,
                interact_background_opacity: 0.0,
                dormant_handle_opacity: 1.0,
                active_handle_opacity: 1.0,
                interact_handle_opacity: 1.0,
                ..egui::style::ScrollStyle::floating()
            }
        } else {
            egui::style::ScrollStyle::default()
        };
    });
}
