use std::sync::atomic::{AtomicU8, Ordering};
pub static CURRENT: AtomicU8 = AtomicU8::new(0);
pub const NAMES: [&str; 5] = ["Deepwater", "Meadow", "Ember", "Rosewater", "Obsidian"];
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: u32,
    pub panel: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub surface: u32,
}
const fn rgb(hex: u32) -> u32 {
    (hex & 255) << 16 | (hex & 0xff00) | (hex >> 16) & 255
}
const fn palette(bg: u32, panel: u32, text: u32, muted: u32, accent: u32, surface: u32) -> Palette {
    Palette {
        bg: rgb(bg),
        panel: rgb(panel),
        text: rgb(text),
        muted: rgb(muted),
        accent: rgb(accent),
        surface: rgb(surface),
    }
}
pub const PALETTES: [Palette; 5] = [
    palette(0x131b23, 0x1f2730, 0xe9f1f7, 0xe7dfc6, 0x2274a5, 0x816c61),
    palette(0xfaffd8, 0xecffb0, 0x54577c, 0x54577c, 0x4a7b9d, 0x9aa899),
    palette(0x0a0903, 0x242012, 0xffeaae, 0xffc100, 0xff8200, 0xff0000),
    palette(0xcbeef3, 0xffedf3, 0x880d1e, 0x880d1e, 0xdd2d4a, 0xf49cbb),
    palette(0x000000, 0x111514, 0xeee0cb, 0xbaa898, 0xbfd7ea, 0x839788),
];
pub const SWATCHES: [[u32; 5]; 5] = [
    [0x2274a5, 0xe7dfc6, 0xe9f1f7, 0x131b23, 0x816c61],
    [0xfaffd8, 0xecffb0, 0x9aa899, 0x54577c, 0x4a7b9d],
    [0x0a0903, 0xff0000, 0xff8200, 0xffc100, 0xffeaae],
    [0x880d1e, 0xdd2d4a, 0xf26a8d, 0xf49cbb, 0xcbeef3],
    [0x000000, 0x839788, 0xeee0cb, 0xbaa898, 0xbfd7ea],
];
pub fn current() -> Palette {
    PALETTES[CURRENT.load(Ordering::Relaxed).min(4) as usize]
}
pub fn set(index: u8) {
    CURRENT.store(index.min(4), Ordering::Relaxed);
}
pub fn swatch(value: u32) -> u32 {
    rgb(value)
}

fn luminance(color: u32) -> f32 {
    let channel = |shift: u32| {
        let c = ((color >> shift) & 255u32) as f32 / 255.;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(0) + 0.7152 * channel(8) + 0.0722 * channel(16)
}
pub fn on_accent() -> u32 {
    let l = luminance(current().accent);
    if (l + 0.05) / 0.05 > 1.05 / (l + 0.05) {
        0
    } else {
        0xffffff
    }
}
pub fn light_surface(color: u32) -> bool {
    luminance(color) > 0.35
}
pub fn readable_on(mut ink: u32, background: u32) -> u32 {
    let background_luminance = luminance(background);
    for _ in 0..16 {
        let foreground = luminance(ink);
        if (foreground.max(background_luminance) + 0.05)
            / (foreground.min(background_luminance) + 0.05)
            >= 4.5
        {
            break;
        }
        ink = (0..3).fold(0, |out, channel| {
            let value = (ink >> (channel * 8)) & 255;
            let value = if background_luminance > 0.35 {
                value * 4 / 5
            } else {
                value + (255 - value) / 4
            };
            out | (value << (channel * 8))
        });
    }
    ink
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn theme_text_has_readable_contrast_on_surfaces() {
        for (index, palette) in PALETTES.iter().enumerate() {
            for ink in [palette.text, palette.muted] {
                let a = luminance(ink);
                let b = luminance(palette.panel);
                let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(contrast >= 4.5, "{} contrast {contrast}", NAMES[index]);
            }
        }
    }
    #[test]
    fn battery_glyphs_are_readable_on_every_theme_surface() {
        for palette in PALETTES {
            for percent in [Some(100), Some(59), Some(19), None] {
                let ink = readable_on(crate::battery::color(percent), palette.panel);
                let a = luminance(ink);
                let b = luminance(palette.panel);
                assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
            }
        }
    }
}
