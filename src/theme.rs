use crate::icon;
use crate::ipc::PopupView;
use crate::settings;
use crate::win;
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Pos2};
use std::sync::Arc;

pub const GLYPH_BLUETOOTH: &str = "\u{e702}";
pub const GLYPH_REFRESH: &str = "\u{e72c}";
pub const GLYPH_SETTINGS: &str = "\u{e713}";
pub const GLYPH_MOUSE: &str = "\u{e962}";
pub const GLYPH_HEADSET: &str = "\u{e95b}";
pub const GLYPH_HEADPHONES: &str = "\u{e7f6}";
pub const GLYPH_KEYBOARD: &str = "\u{e92e}";
pub const GLYPH_CONTROLLER: &str = "\u{e7fc}";
pub const GLYPH_BATTERY: &str = "\u{e83f}";
pub const GLYPH_LIGHTNING_BOLT: &str = "\u{e945}";
pub const GLYPH_BATTERY_UNKNOWN: &str = "\u{e996}";
pub const GLYPH_BATTERY_PAGE: &str = "\u{e850}";
pub const GLYPH_TRAY_PAGE: &str = "\u{e7f4}";
pub const GLYPH_APPEARANCE_PAGE: &str = "\u{e790}";
pub const GLYPH_CHECK: &str = "\u{e73e}";
pub const GLYPH_CHEVRON_UP: &str = "\u{e70e}";
pub const GLYPH_CHEVRON_DOWN: &str = "\u{e70d}";

const COLOR_WINDOW: i32 = 5;
const COLOR_WINDOW_TEXT: i32 = 8;
const COLOR_HIGHLIGHT: i32 = 13;
const COLOR_HIGHLIGHT_TEXT: i32 = 14;
const COLOR_BUTTON_FACE: i32 = 15;
const COLOR_GRAY_TEXT: i32 = 17;
const COLOR_BUTTON_TEXT: i32 = 18;

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub dark: bool,
    pub high_contrast: bool,
    pub popup_background: Color32,
    pub popup_surface: Color32,
    pub popup_border: Color32,
    pub popup_divider: Color32,
    pub popup_foreground: Color32,
    pub popup_muted: Color32,
    pub popup_accent: Color32,
    pub popup_logo: Color32,
    pub popup_action: Color32,
    pub popup_action_hover: Color32,
    pub popup_action_border: Color32,
    pub popup_track: Color32,
    pub popup_footer: Color32,
    pub popup_plain_bar: Color32,
    pub base: Color32,
    pub sidebar: Color32,
    pub surface: Color32,
    pub foreground: Color32,
    pub secondary: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub navigation_selected: Color32,
    pub navigation_selected_text: Color32,
    pub control: Color32,
    pub control_hover: Color32,
    pub preview_frame: Color32,
    pub primary: Color32,
    pub primary_border: Color32,
    pub primary_text: Color32,
    pub warning: Color32,
}

const fn rgb(red: u8, green: u8, blue: u8) -> Color32 {
    Color32::from_rgb(red, green, blue)
}

fn system(index: i32) -> Color32 {
    let [red, green, blue] = win::system_color(index);
    rgb(red, green, blue)
}

fn blend(top: Color32, bottom: Color32, amount: f32) -> Color32 {
    let mix = |top: u8, bottom: u8| (f32::from(top) * amount + f32::from(bottom) * (1.0 - amount)).round() as u8;
    rgb(mix(top.r(), bottom.r()), mix(top.g(), bottom.g()), mix(top.b(), bottom.b()))
}

impl Theme {
    pub fn resolve(theme: &str) -> Self {
        if win::high_contrast() {
            Self::high_contrast()
        } else if settings::is_dark_theme(theme) {
            Self::dark()
        } else {
            Self::light()
        }
    }

    pub fn dark() -> Self {
        let popup_surface = rgb(0x26, 0x28, 0x2c);
        let popup_border = rgb(0x5c, 0x60, 0x67);
        Self {
            dark: true,
            high_contrast: false,
            popup_background: rgb(0x16, 0x16, 0x16),
            popup_surface,
            popup_border,
            popup_divider: blend(popup_border, popup_surface, 0.65),
            popup_foreground: rgb(0xf2, 0xf3, 0xf5),
            popup_muted: rgb(0xc3, 0xc9, 0xd2),
            popup_accent: rgb(0x5a, 0xa9, 0xff),
            popup_logo: rgb(0x1f, 0x57, 0x90),
            popup_action: rgb(0x30, 0x34, 0x3a),
            popup_action_hover: rgb(0x3c, 0x41, 0x48),
            popup_action_border: rgb(0x5c, 0x60, 0x67),
            popup_track: rgb(0x55, 0x5b, 0x64),
            popup_footer: rgb(0xff, 0xd3, 0x80),
            popup_plain_bar: rgb(0xa2, 0xa9, 0xb1),
            base: rgb(22, 22, 22),
            sidebar: rgb(18, 18, 18),
            surface: rgb(34, 34, 34),
            foreground: rgb(220, 220, 220),
            secondary: rgb(192, 192, 192),
            border: rgb(78, 78, 78),
            accent: rgb(22, 133, 220),
            navigation_selected: rgb(25, 66, 100),
            navigation_selected_text: rgb(220, 220, 220),
            control: rgb(45, 45, 45),
            control_hover: rgb(56, 56, 56),
            preview_frame: rgb(20, 20, 20),
            primary: rgb(0, 105, 190),
            primary_border: rgb(38, 151, 235),
            primary_text: Color32::WHITE,
            warning: rgb(255, 205, 86),
        }
    }

    pub fn light() -> Self {
        let popup_surface = rgb(0xff, 0xff, 0xff);
        let popup_border = rgb(0xbe, 0xc7, 0xd0);
        Self {
            dark: false,
            high_contrast: false,
            popup_background: rgb(0xf4, 0xf6, 0xf8),
            popup_surface,
            popup_border,
            popup_divider: blend(popup_border, popup_surface, 0.65),
            popup_foreground: rgb(0x1c, 0x22, 0x2a),
            popup_muted: rgb(0x50, 0x5b, 0x68),
            popup_accent: rgb(0x00, 0x69, 0xc8),
            popup_logo: rgb(0x00, 0x69, 0xc8),
            popup_action: rgb(0xe5, 0xeb, 0xf1),
            popup_action_hover: rgb(0xd6, 0xde, 0xe6),
            popup_action_border: rgb(0xaa, 0xb6, 0xc2),
            popup_track: rgb(0xca, 0xd3, 0xdd),
            popup_footer: rgb(0x7e, 0x48, 0x00),
            popup_plain_bar: rgb(0x68, 0x72, 0x7d),
            base: rgb(246, 248, 248),
            sidebar: rgb(237, 241, 241),
            surface: rgb(251, 251, 251),
            foreground: rgb(24, 30, 31),
            secondary: rgb(82, 90, 92),
            border: rgb(190, 198, 199),
            accent: rgb(22, 133, 220),
            navigation_selected: rgb(203, 228, 250),
            navigation_selected_text: rgb(24, 30, 31),
            control: rgb(255, 255, 255),
            control_hover: rgb(240, 243, 245),
            preview_frame: rgb(232, 232, 232),
            primary: rgb(0, 105, 190),
            primary_border: rgb(38, 151, 235),
            primary_text: Color32::WHITE,
            warning: rgb(170, 0, 0),
        }
    }

    fn high_contrast() -> Self {
        let window = system(COLOR_WINDOW);
        let text = system(COLOR_WINDOW_TEXT);
        let highlight = system(COLOR_HIGHLIGHT);
        let highlight_text = system(COLOR_HIGHLIGHT_TEXT);
        let control = system(COLOR_BUTTON_FACE);
        let brightness = u32::from(window.r()) * 299 + u32::from(window.g()) * 587 + u32::from(window.b()) * 114;
        Self {
            dark: brightness < 128_000,
            high_contrast: true,
            popup_background: window,
            popup_surface: window,
            popup_border: text,
            popup_divider: text,
            popup_foreground: text,
            popup_muted: text,
            popup_accent: highlight,
            popup_logo: highlight,
            popup_action: control,
            popup_action_hover: control,
            popup_action_border: system(COLOR_BUTTON_TEXT),
            popup_track: system(COLOR_GRAY_TEXT),
            popup_footer: text,
            popup_plain_bar: highlight,
            base: window,
            sidebar: control,
            surface: window,
            foreground: text,
            secondary: text,
            border: text,
            accent: highlight,
            navigation_selected: highlight,
            navigation_selected_text: highlight_text,
            control,
            control_hover: control,
            preview_frame: window,
            primary: highlight,
            primary_border: highlight,
            primary_text: highlight_text,
            warning: text,
        }
    }

    pub fn battery_color(&self, percent: Option<u8>, view: &PopupView) -> Color32 {
        if self.high_contrast {
            return self.popup_accent;
        }
        if !view.color_bars {
            return self.popup_plain_bar;
        }
        let remaining = percent.unwrap_or(0);
        if i32::from(remaining) <= view.low_threshold {
            let [red, green, blue] = icon::color_for_percent(remaining, view.critical_threshold, view.low_threshold);
            rgb(red, green, blue)
        } else {
            self.popup_accent
        }
    }
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn icons(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("icons".into()))
}

pub fn load_fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let directory = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_owned()) + "\\Fonts\\";
    let mut load = |name: &str, files: &[&str]| {
        for file in files {
            if let Ok(bytes) = std::fs::read(format!("{directory}{file}")) {
                fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
                return true;
            }
        }
        false
    };
    let has_regular = load("segoe", &["segoeui.ttf"]);
    let has_semibold = load("segoe-semibold", &["seguisb.ttf", "segoeuib.ttf"]);
    let has_icons = load("segoe-icons", &["SegoeIcons.ttf", "segmdl2.ttf"]);

    let mut proportional = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if has_regular {
        proportional.insert(0, "segoe".to_owned());
    }
    let mut strong = proportional.clone();
    if has_semibold {
        strong.insert(0, "segoe-semibold".to_owned());
    }
    let mut symbols = proportional.clone();
    if has_icons {
        symbols.insert(0, "segoe-icons".to_owned());
    }
    fonts.families.insert(FontFamily::Proportional, proportional);
    fonts.families.insert(FontFamily::Name("semibold".into()), strong);
    fonts.families.insert(FontFamily::Name("icons".into()), symbols);
    fonts
}

pub fn paint_icon(painter: &egui::Painter, center: Pos2, glyph: &str, size: f32, color: Color32) {
    let galley = painter.layout_no_wrap(glyph.to_owned(), icons(size), color);
    let ink = galley.mesh_bounds;
    let offset = if ink.is_finite() && ink.width() > 0.0 { ink.center().to_vec2() } else { galley.size() / 2.0 };
    painter.galley(center - offset, galley, color);
}

pub fn truncated(painter: &egui::Painter, text: &str, font: FontId, color: Color32, width: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    painter.layout_job(job)
}

pub fn wrapped(painter: &egui::Painter, text: &str, font: FontId, color: Color32, width: f32) -> Arc<egui::Galley> {
    painter.layout_job(egui::text::LayoutJob::simple(text.to_owned(), font, color, width.max(1.0)))
}

pub fn device_glyph(name: &str) -> &'static str {
    let name = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|word| name.contains(word));
    if has(&["mouse", "superlight", "g305", "pulsefire"]) {
        GLYPH_MOUSE
    } else if has(&["earbud", "airpod", "buds"]) {
        GLYPH_HEADPHONES
    } else if has(&["headset", "hyperx"]) {
        GLYPH_HEADSET
    } else if has(&["headphone"]) {
        GLYPH_HEADPHONES
    } else if has(&["controller", "gamepad", "xbox"]) {
        GLYPH_CONTROLLER
    } else if has(&["keyboard"]) {
        GLYPH_KEYBOARD
    } else {
        GLYPH_BLUETOOTH
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(color_bars: bool) -> PopupView {
        PopupView { color_bars, low_threshold: 25, critical_threshold: 15, ..Default::default() }
    }

    #[test]
    fn battery_colours_follow_thresholds_and_the_colour_setting() {
        let theme = Theme::dark();
        assert_eq!(theme.battery_color(Some(80), &view(true)), theme.popup_accent);
        assert_eq!(theme.battery_color(Some(25), &view(true)), rgb(240, 210, 0));
        assert_eq!(theme.battery_color(Some(15), &view(true)), rgb(220, 40, 40));
        assert_eq!(theme.battery_color(None, &view(true)), rgb(220, 40, 40));
        assert_eq!(theme.battery_color(Some(5), &view(false)), theme.popup_plain_bar);
    }

    #[test]
    fn device_glyphs_match_the_original_name_rules() {
        assert_eq!(device_glyph("PRO X SUPERLIGHT 2c"), GLYPH_MOUSE);
        assert_eq!(device_glyph("HyperX Cloud III S Wireless"), GLYPH_HEADSET);
        assert_eq!(device_glyph("Galaxy Buds"), GLYPH_HEADPHONES);
        assert_eq!(device_glyph("Xbox Wireless Controller"), GLYPH_CONTROLLER);
        assert_eq!(device_glyph("MX Keyboard"), GLYPH_KEYBOARD);
        assert_eq!(device_glyph("PRO X3 SUPERSTRIKE"), GLYPH_BLUETOOTH);
    }
}
