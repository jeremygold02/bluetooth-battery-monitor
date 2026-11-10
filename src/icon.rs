use crate::settings::Settings;
use ab_glyph::{Font, FontVec, PxScale, ScaleFont};

pub const ICON_SIZE: usize = 32;
pub const NORMAL_COLOR: [u8; 3] = [0, 120, 215];
pub const WARNING_COLOR: [u8; 3] = [240, 210, 0];
pub const CRITICAL_COLOR: [u8; 3] = [220, 40, 40];
const UNKNOWN_COLOR: [u8; 3] = [64, 64, 64];
const ERROR_COLOR: [u8; 3] = [210, 46, 46];
const WHITE: [u8; 3] = [255; 3];
const BLACK: [u8; 3] = [0; 3];
const MEASURE_SIZE: f32 = 1000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IconContent {
    Percent(u8),
    Unknown,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconStyle {
    pub show_box: bool,
    pub custom_box: Option<[u8; 3]>,
    pub text: [u8; 3],
    pub critical: i32,
    pub low: i32,
}

impl IconStyle {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            show_box: settings.show_tray_box,
            custom_box: settings.use_custom_tray_box_color.then(|| settings.tray_box_rgb()),
            text: settings.tray_text_rgb(),
            critical: settings.critical_threshold,
            low: settings.low_threshold,
        }
    }

    pub fn box_color(&self, percent: u8) -> [u8; 3] {
        self.custom_box.unwrap_or_else(|| color_for_percent(percent, self.critical, self.low))
    }
}

pub fn color_for_percent(percent: u8, critical: i32, low: i32) -> [u8; 3] {
    let percent = i32::from(percent);
    if percent <= critical {
        CRITICAL_COLOR
    } else if percent <= low {
        WARNING_COLOR
    } else {
        NORMAL_COLOR
    }
}

pub struct IconFonts {
    semibold: Option<FontVec>,
    regular: Option<FontVec>,
}

impl IconFonts {
    pub fn load() -> Self {
        let directory = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_owned()) + "\\Fonts\\";
        let load = |files: &[&str]| {
            files.iter().find_map(|file| {
                let bytes = std::fs::read(format!("{directory}{file}")).ok()?;
                FontVec::try_from_vec(bytes).ok()
            })
        };
        Self { semibold: load(&["seguisb.ttf", "segoeui.ttf"]), regular: load(&["segoeui.ttf", "micross.ttf"]) }
    }

    #[cfg(test)]
    fn none() -> Self {
        Self { semibold: None, regular: None }
    }
}

pub fn render(content: IconContent, style: &IconStyle, fonts: &IconFonts) -> Vec<u8> {
    let (background, foreground, text, show_box) = match content {
        IconContent::Percent(percent) => {
            let percent = percent.min(100);
            let background = style.box_color(percent);
            let foreground = if style.show_box && background == WARNING_COLOR { BLACK } else { style.text };
            (background, foreground, percent.to_string(), style.show_box)
        }
        IconContent::Unknown => (UNKNOWN_COLOR, WHITE, "...".to_owned(), true),
        IconContent::Error => (ERROR_COLOR, WHITE, "!".to_owned(), true),
    };
    let fill = if show_box { [background[0], background[1], background[2], 255] } else { [0; 4] };
    let mut rgba: Vec<u8> = (0..ICON_SIZE * ICON_SIZE).flat_map(|_| fill).collect();
    let (bounds, squeeze) = match text.as_str() {
        "!" => ([4.0, 4.0, 24.0, 24.0], 1.0),
        "100" => ([3.0, 6.0, 26.0, 18.0], 1.0),
        text if text.len() >= 3 => ([1.0, 3.0, 30.0, 25.0], 1.0),
        text if text.len() == 2 => ([1.0, 6.0, 30.0, 20.0], 0.92),
        _ => ([1.0, 6.0, 30.0, 20.0], 1.0),
    };
    let font = if text == "!" { fonts.regular.as_ref() } else { fonts.semibold.as_ref() };
    if let Some(font) = font {
        draw_fitted_text(&mut rgba, font, &text, bounds, squeeze, foreground);
    }
    rgba
}

fn outline_text(font: &FontVec, text: &str, size: f32, origin: (f32, f32)) -> Vec<ab_glyph::OutlinedGlyph> {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut x = origin.0;
    let mut previous = None;
    let mut glyphs = Vec::new();
    for character in text.chars() {
        let id = font.glyph_id(character);
        if let Some(previous) = previous {
            x += scaled.kern(previous, id);
        }
        let glyph = id.with_scale_and_position(size, ab_glyph::point(x, origin.1));
        if let Some(outlined) = font.outline_glyph(glyph) {
            glyphs.push(outlined);
        }
        x += scaled.h_advance(id);
        previous = Some(id);
    }
    glyphs
}

fn draw_fitted_text(rgba: &mut [u8], font: &FontVec, text: &str, bounds: [f32; 4], squeeze: f32, color: [u8; 3]) {
    let measured = outline_text(font, text, MEASURE_SIZE, (0.0, 0.0));
    let Some(first) = measured.first() else { return };
    let mut ink = first.px_bounds();
    for glyph in &measured[1..] {
        let glyph_bounds = glyph.px_bounds();
        ink.min.x = ink.min.x.min(glyph_bounds.min.x);
        ink.min.y = ink.min.y.min(glyph_bounds.min.y);
        ink.max.x = ink.max.x.max(glyph_bounds.max.x);
        ink.max.y = ink.max.y.max(glyph_bounds.max.y);
    }
    if ink.width() <= 0.0 || ink.height() <= 0.0 {
        return;
    }
    let [left, top, width, height] = bounds;
    let factor = (width / ink.width()).min(height / ink.height()) * squeeze;
    let origin = (
        left + width / 2.0 - (ink.min.x + ink.max.x) / 2.0 * factor,
        top + height / 2.0 - (ink.min.y + ink.max.y) / 2.0 * factor,
    );
    for glyph in outline_text(font, text, MEASURE_SIZE * factor, origin) {
        let glyph_bounds = glyph.px_bounds();
        glyph.draw(|x, y, coverage| {
            let pixel_x = glyph_bounds.min.x as i32 + x as i32;
            let pixel_y = glyph_bounds.min.y as i32 + y as i32;
            let size = ICON_SIZE as i32;
            if pixel_x < 0 || pixel_y < 0 || pixel_x >= size || pixel_y >= size {
                return;
            }
            let offset = (pixel_y as usize * ICON_SIZE + pixel_x as usize) * 4;
            let coverage = coverage.clamp(0.0, 1.0);
            if rgba[offset + 3] == 255 {
                for channel in 0..3 {
                    let existing = f32::from(rgba[offset + channel]);
                    let blended = existing + (f32::from(color[channel]) - existing) * coverage;
                    rgba[offset + channel] = blended.round() as u8;
                }
            } else {
                let alpha = (coverage * 255.0).round() as u8;
                if alpha > rgba[offset + 3] {
                    rgba[offset..offset + 4].copy_from_slice(&[color[0], color[1], color[2], alpha]);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> IconStyle {
        IconStyle { show_box: true, custom_box: None, text: WHITE, critical: 15, low: 25 }
    }

    fn ink(rgba: &[u8], background: [u8; 4]) -> Vec<(usize, usize)> {
        (0..ICON_SIZE * ICON_SIZE)
            .filter(|index| rgba[index * 4..index * 4 + 4] != background)
            .map(|index| (index % ICON_SIZE, index / ICON_SIZE))
            .collect()
    }

    #[test]
    fn backgrounds_follow_the_thresholds_and_custom_colour() {
        let fonts = IconFonts::none();
        let solid = |rgba: Vec<u8>, color: [u8; 3]| {
            rgba.chunks(4).all(|pixel| pixel == [color[0], color[1], color[2], 255])
        };
        assert!(solid(render(IconContent::Percent(53), &style(), &fonts), NORMAL_COLOR));
        assert!(solid(render(IconContent::Percent(25), &style(), &fonts), WARNING_COLOR));
        assert!(solid(render(IconContent::Percent(15), &style(), &fonts), CRITICAL_COLOR));
        assert!(solid(render(IconContent::Unknown, &style(), &fonts), UNKNOWN_COLOR));
        assert!(solid(render(IconContent::Error, &style(), &fonts), ERROR_COLOR));
        let custom = IconStyle { custom_box: Some([0, 88, 83]), ..style() };
        assert!(solid(render(IconContent::Percent(10), &custom, &fonts), [0, 88, 83]));
        let hidden = IconStyle { show_box: false, ..style() };
        assert!(render(IconContent::Percent(10), &hidden, &fonts).iter().all(|byte| *byte == 0));
    }

    #[test]
    fn text_fills_and_stays_inside_its_bounds() {
        let fonts = IconFonts::load();
        if fonts.semibold.is_none() || fonts.regular.is_none() {
            return;
        }
        let cases = [
            (IconContent::Percent(5), 8),
            (IconContent::Percent(53), 19),
            (IconContent::Percent(100), 22),
            (IconContent::Unknown, 20),
            (IconContent::Error, 3),
        ];
        for (content, min_width) in cases {
            let rgba = render(content, &style(), &fonts);
            let background = [rgba[0], rgba[1], rgba[2], rgba[3]];
            let drawn = ink(&rgba, background);
            assert!(!drawn.is_empty(), "{content:?} drew no text");
            let left = drawn.iter().map(|p| p.0).min().unwrap();
            let right = drawn.iter().map(|p| p.0).max().unwrap();
            let top = drawn.iter().map(|p| p.1).min().unwrap();
            let bottom = drawn.iter().map(|p| p.1).max().unwrap();
            assert!(left >= 1 && right <= 30 && top >= 3 && bottom <= 28, "{content:?} leaves its bounds");
            assert!(right - left + 1 >= min_width, "{content:?} is only {} px wide", right - left + 1);
            let centre = (left + right) as f32 / 2.0;
            assert!((centre - 15.5).abs() <= 1.0, "{content:?} is off centre at {centre}");
        }
    }

    #[test]
    fn warning_backgrounds_use_black_text_and_hidden_boxes_keep_only_text() {
        let fonts = IconFonts::load();
        if fonts.semibold.is_none() {
            return;
        }
        let warning = render(IconContent::Percent(21), &style(), &fonts);
        assert!(warning.chunks(4).any(|pixel| pixel == [0, 0, 0, 255]));
        let custom_text = IconStyle { text: [10, 200, 30], show_box: false, ..style() };
        let hidden = render(IconContent::Percent(60), &custom_text, &fonts);
        assert!(hidden.chunks(4).any(|pixel| pixel == [10, 200, 30, 255]));
        assert!(hidden.chunks(4).all(|pixel| pixel[3] == 0 || pixel[..3] == [10, 200, 30]));
    }

    #[test]
    #[ignore = "writes tray icon previews next to the screenshots"]
    fn writes_tray_icon_previews() {
        let fonts = IconFonts::load();
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("screenshots");
        std::fs::create_dir_all(&directory).unwrap();
        let hidden = IconStyle { show_box: false, ..style() };
        let teal = IconStyle { custom_box: Some([0, 194, 186]), text: BLACK, ..style() };
        let samples = [
            (IconContent::Percent(53), style()),
            (IconContent::Percent(100), style()),
            (IconContent::Percent(21), style()),
            (IconContent::Percent(8), style()),
            (IconContent::Unknown, style()),
            (IconContent::Error, style()),
            (IconContent::Percent(79), hidden),
            (IconContent::Percent(64), teal),
        ];
        let scale = 6;
        let gap = 8;
        let width = samples.len() * (ICON_SIZE * scale + gap) + gap;
        let height = ICON_SIZE * scale + gap * 2;
        let stride = (width * 3).div_ceil(4) * 4;
        let mut sheet = vec![0x20u8; stride * height];
        for (index, (content, style)) in samples.iter().enumerate() {
            let rgba = render(*content, style, &fonts);
            let origin_x = gap + index * (ICON_SIZE * scale + gap);
            for y in 0..ICON_SIZE * scale {
                for x in 0..ICON_SIZE * scale {
                    let source = ((y / scale) * ICON_SIZE + x / scale) * 4;
                    let target = (height - 1 - gap - y) * stride + (origin_x + x) * 3;
                    let alpha = u32::from(rgba[source + 3]);
                    for channel in 0..3 {
                        let value = (u32::from(rgba[source + 2 - channel]) * alpha + 0x20 * (255 - alpha)) / 255;
                        sheet[target + channel] = value as u8;
                    }
                }
            }
        }
        let mut bmp = Vec::with_capacity(54 + sheet.len());
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&(54 + sheet.len() as u32).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&(width as i32).to_le_bytes());
        bmp.extend_from_slice(&(height as i32).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(&sheet);
        std::fs::write(directory.join("tray-icons.bmp"), bmp).unwrap();
    }
}
