use std::sync::atomic::{AtomicU8, Ordering};
pub static CURRENT: AtomicU8 = AtomicU8::new(0);
pub const NAMES: [&str; 4] = ["Deepwater", "Meadow", "Ember", "Rosewater"];
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
pub const PALETTES: [Palette; 4] = [
    palette(0x131b23, 0x1f2730, 0xe9f1f7, 0xe7dfc6, 0x2274a5, 0x816c61),
    palette(0xfaffd8, 0xecffb0, 0x54577c, 0x54577c, 0x4a7b9d, 0x9aa899),
    palette(0x0a0903, 0x242012, 0xffeaae, 0xffc100, 0xff8200, 0xff0000),
    palette(0xcbeef3, 0xf49cbb, 0x880d1e, 0x880d1e, 0xdd2d4a, 0xf26a8d),
];
pub const SWATCHES: [[u32; 5]; 4] = [
    [0x2274a5, 0xe7dfc6, 0xe9f1f7, 0x131b23, 0x816c61],
    [0xfaffd8, 0xecffb0, 0x9aa899, 0x54577c, 0x4a7b9d],
    [0x0a0903, 0xff0000, 0xff8200, 0xffc100, 0xffeaae],
    [0x880d1e, 0xdd2d4a, 0xf26a8d, 0xf49cbb, 0xcbeef3],
];
pub fn current() -> Palette {
    PALETTES[CURRENT.load(Ordering::Relaxed).min(3) as usize]
}
pub fn set(index: u8) {
    CURRENT.store(index.min(3), Ordering::Relaxed);
}
pub fn swatch(value: u32) -> u32 {
    rgb(value)
}

pub fn on_accent() -> u32 {
    if CURRENT.load(Ordering::Relaxed) == 2 {
        rgb(0x0a0903)
    } else {
        0xffffff
    }
}
