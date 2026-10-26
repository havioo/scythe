use crate::{Action, State, device::wide};
use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::*,
    UI::{HiDpi::*, Shell::*, WindowsAndMessaging::*},
};

// All HWNDs and interaction state live exclusively on the message-loop thread.
static STATE: OnceLock<Arc<Mutex<State>>> = OnceLock::new();
static REFRESH: OnceLock<Arc<AtomicBool>> = OnceLock::new();
static COMMANDS: OnceLock<Sender<Action>> = OnceLock::new();
static GLOW: OnceLock<Mutex<(String, u32, Vec<u8>)>> = OnceLock::new();
#[link(name = "user32")]
unsafe extern "system" {
    fn SetCapture(hwnd: HWND) -> HWND;
    fn ReleaseCapture() -> i32;
    fn SetFocus(hwnd: HWND) -> HWND;
}
#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: HWND,
        attribute: u32,
        value: *const std::ffi::c_void,
        size: u32,
    ) -> i32;
}
const TRAY: u32 = WM_APP + 1;
const SHOW: u32 = WM_APP + 2;
const W: i32 = 540;
const H: i32 = 754;
static PAGE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static DRAFT: Mutex<Option<[i8; 10]>> = Mutex::new(None);
static FEATURE_FOCUS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct Ui {
    tip: HWND,
    icon: HICON,
    baseline: Option<u64>,
    drag: i32,
    focus: i32,
    mic: Option<u8>,
    game: Option<u8>,
    balance: Option<u8>,
    dpi: u32,
    taskbar_created: u32,
}
unsafe fn ui(hwnd: HWND) -> &'static mut Ui {
    unsafe { &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui) }
}
fn state() -> State {
    STATE
        .get()
        .unwrap()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}
fn send(action: Action) {
    if let Some(tx) = COMMANDS.get() {
        let _ = tx.send(action);
    }
}
fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}
unsafe fn fill(dc: HDC, r: RECT, c: u32) {
    unsafe {
        crate::render::fill(dc, r, c);
    }
}
unsafe fn round(dc: HDC, r: RECT, c: u32, radius: i32) {
    unsafe {
        crate::render::rounded(dc, r, c, radius);
    }
}
// Tonal surfaces provide separation without adding visual noise to the card gutters.
unsafe fn card(dc: HDC, r: RECT, radius: i32) {
    unsafe {
        round(dc, r, crate::theme::current().panel, radius);
    }
}
#[allow(clippy::too_many_arguments)] // Mirrors the native text rectangle and typography arguments.
unsafe fn text(dc: HDC, x: i32, y: i32, w: i32, h: i32, s: &str, c: u32, size: i32, weight: i32) {
    unsafe {
        text_layout(dc, x, y, w, h, s, c, size, weight, DT_LEFT);
    }
}
#[allow(clippy::too_many_arguments)]
unsafe fn text_layout(
    dc: HDC,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    s: &str,
    c: u32,
    size: i32,
    weight: i32,
    align: u32,
) {
    unsafe {
        crate::render::text(dc, rect(x, y, w, h), s, c, size, weight, align == DT_RIGHT);
    }
}
#[allow(clippy::too_many_arguments)] // Same rectangle/typography contract as native text drawing.
unsafe fn value(dc: HDC, x: i32, y: i32, w: i32, h: i32, s: &str, c: u32, size: i32, weight: i32) {
    unsafe {
        text_layout(dc, x, y, w, h, s, c, size, weight, DT_RIGHT);
    }
}
unsafe fn dib(dc: HDC, w: i32, h: i32) -> (HBITMAP, *mut u8) {
    unsafe {
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = w;
        info.bmiHeader.biHeight = -h;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits = null_mut();
        let bm = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        (bm, bits.cast())
    }
}
// Rasterize the actual percentage glyphs to a mask, blur it in two passes, then
// composite into the card. This is a real soft glyph halo, without layered text tricks.
unsafe fn percentage(dc: HDC, value: &str, color: u32) {
    unsafe {
        let (w, h) = (280usize, 142usize);
        let mem = CreateCompatibleDC(dc);
        let (bm, bits) = dib(dc, w as i32, h as i32);
        if bm.is_null() {
            DeleteDC(mem);
            return;
        }
        let old = SelectObject(mem, bm);
        let mut cached = GLOW
            .get_or_init(|| Mutex::new((String::new(), 0, Vec::new())))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if cached.0 != format!("{value}:{}", crate::theme::current().panel) || cached.1 != color {
            fill(mem, rect(0, 0, w as i32, h as i32), 0);
            text(mem, 24, 12, 250, 110, value, 0xffffff, 76, 600);
            GdiFlush();
            let pixels = std::slice::from_raw_parts_mut(bits, w * h * 4);
            let mut mask = vec![0f32; w * h];
            for i in 0..w * h {
                mask[i] = pixels[i * 4] as f32 / 255.;
            }
            let mut temp = vec![0f32; w * h];
            for _ in 0..3 {
                for y in 0..h {
                    for x in 0..w {
                        let mut n = 0.;
                        for dx in -6isize..=6 {
                            let xx = x as isize + dx;
                            if xx >= 0 && xx < w as isize {
                                n += mask[y * w + xx as usize];
                            }
                        }
                        temp[y * w + x] = n / 13.;
                    }
                }
                for y in 0..h {
                    for x in 0..w {
                        let mut n = 0.;
                        for dy in -6isize..=6 {
                            let yy = y as isize + dy;
                            if yy >= 0 && yy < h as isize {
                                n += temp[yy as usize * w + x];
                            }
                        }
                        mask[y * w + x] = n / 13.;
                    }
                }
            }
            for i in 0..w * h {
                let a = (mask[i] * 0.68).min(0.75);
                for ch in 0..3 {
                    // DIB bytes are BGR; COLORREF is 0x00BBGGRR.
                    let shift = (2 - ch) * 8;
                    let base = ((crate::theme::current().panel >> shift) & 255) as f32;
                    let light = ((color >> shift) & 255) as f32;
                    pixels[i * 4 + ch] = (base + (light - base) * a) as u8;
                }
                pixels[i * 4 + 3] = 255;
            }
            cached.0 = format!("{value}:{}", crate::theme::current().panel);
            cached.1 = color;
            cached.2 = pixels.to_vec();
        } else {
            GdiFlush();
            std::slice::from_raw_parts_mut(bits, w * h * 4).copy_from_slice(&cached.2);
        }
        crate::render::image(
            dc,
            rect(31, 87, w as i32, h as i32),
            w as i32,
            h as i32,
            &cached.2,
        );
        SelectObject(mem, old);
        DeleteObject(bm);
        DeleteDC(mem);
        text(dc, 55, 99, 250, 110, value, color, 76, 600);
    }
}
fn percent(v: Option<u8>) -> String {
    v.map(|v| format!("{v}%")).unwrap_or_else(|| "—".into())
}
unsafe fn slider(dc: HDC, y: i32, v: Option<u8>, focus: bool) {
    unsafe {
        round(dc, rect(48, y, 444, 6), crate::theme::current().surface, 6);
        if let Some(v) = v {
            let x = 48 + 444 * (v.min(100) as i32) / 100;
            round(
                dc,
                rect(48, y, x - 48, 6),
                crate::theme::current().accent,
                6,
            );
            round(
                dc,
                rect(x - 7, y - 4, 14, 14),
                crate::theme::current().text,
                14,
            );
        }
        if focus {
            text(
                dc,
                48,
                y + 10,
                440,
                17,
                "Left / Right adjust volume",
                crate::theme::current().accent,
                11,
                400,
            );
        }
    }
}
unsafe fn paint(
    dc: HDC,
    s: &State,
    compact: bool,
    focus: i32,
    mic: Option<u8>,
    game: Option<u8>,
    balance: Option<u8>,
) {
    unsafe {
        crate::theme::set(s.theme);
        fill(
            dc,
            rect(
                0,
                0,
                if compact { 290 } else { W },
                if compact { 100 } else { H },
            ),
            crate::theme::current().bg,
        );
        let effective = s.hardware_mic_muted.unwrap_or(false) || s.audio.microphone_muted;
        if compact {
            round(dc, rect(1, 1, 288, 98), crate::theme::current().panel, 16);
            text(
                dc,
                16,
                10,
                248,
                20,
                "SCYTHE  /  STATUS",
                crate::theme::current().accent,
                11,
                600,
            );
            text(
                dc,
                16,
                34,
                100,
                42,
                &percent(s.battery),
                crate::battery::color(s.battery),
                30,
                600,
            );
            text(
                dc,
                121,
                35,
                152,
                25,
                if s.hardware_mic_muted == Some(true) {
                    "Muted on headset"
                } else if effective {
                    "Microphone muted"
                } else {
                    "Microphone live"
                },
                if effective {
                    crate::theme::current().accent
                } else {
                    crate::theme::current().text
                },
                13,
                400,
            );
            text(
                dc,
                121,
                58,
                152,
                18,
                &format!(
                    "Game {}{}",
                    percent(s.audio.speaker),
                    if s.audio.speaker_muted {
                        " · muted"
                    } else {
                        ""
                    }
                ),
                crate::theme::current().muted,
                11,
                400,
            );
            text(
                dc,
                16,
                79,
                258,
                16,
                &s.status,
                crate::theme::current().muted,
                11,
                400,
            );
            return;
        }
        round(
            dc,
            rect(426, 14, 38, 30),
            if focus == 8 {
                crate::theme::current().accent
            } else {
                crate::theme::current().surface
            },
            10,
        );
        round(
            dc,
            rect(472, 14, 38, 30),
            if focus == 9 {
                crate::theme::current().accent
            } else {
                crate::theme::current().surface
            },
            10,
        );
        text(
            dc,
            438,
            14,
            20,
            28,
            "\u{2212}",
            crate::theme::current().text,
            18,
            600,
        );
        text(
            dc,
            483,
            14,
            20,
            28,
            "\u{00d7}",
            crate::theme::current().text,
            18,
            600,
        );
        text(
            dc,
            30,
            17,
            360,
            22,
            "SCYTHE",
            crate::theme::current().accent,
            13,
            700,
        );
        let page = PAGE.load(Ordering::Relaxed);
        for (i, label) in ["Overview", "Equalizer", "Headset", "Themes"]
            .iter()
            .enumerate()
        {
            round(
                dc,
                rect(30 + i as i32 * 122, 48, 114, 25),
                if page as usize == i {
                    crate::theme::current().accent
                } else {
                    crate::theme::current().panel
                },
                10,
            );
            text(
                dc,
                39 + i as i32 * 122,
                49,
                102,
                23,
                label,
                if page as usize == i {
                    crate::theme::on_accent()
                } else {
                    crate::theme::current().text
                },
                12,
                600,
            );
        }
        if page > 0 {
            paint_features(dc, s, page);
            return;
        }
        card(dc, rect(30, 82, 480, 154), 24);
        percentage(dc, &percent(s.battery), crate::battery::color(s.battery));
        text(
            dc,
            48,
            91,
            300,
            22,
            "BATTERY",
            crate::theme::current().muted,
            11,
            600,
        );
        text(
            dc,
            320,
            132,
            165,
            25,
            match s.charging {
                Some(true) => "Charging",
                Some(false) => "On battery",
                None => "Awaiting headset",
            },
            crate::theme::current().text,
            16,
            600,
        );
        let remaining = s
            .estimated_minutes
            .map(|m| format!("About {}h {}m left", m / 60, m % 60))
            .unwrap_or_else(|| {
                if s.battery.is_some() {
                    "Learning battery life".into()
                } else {
                    "Connect to see battery".into()
                }
            });
        text(
            dc,
            320,
            160,
            165,
            22,
            &remaining,
            crate::theme::current().muted,
            12,
            400,
        );
        text(
            dc,
            48,
            208,
            248,
            22,
            &s.status,
            crate::theme::current().muted,
            13,
            400,
        );
        text(
            dc,
            320,
            208,
            172,
            24,
            &s.preset
                .map(|v| format!("Sound preset {v}"))
                .unwrap_or_else(|| "Sound preset unavailable".into()),
            crate::theme::current().muted,
            12,
            400,
        );
        card(dc, rect(30, 248, 480, 200), 20);
        text(
            dc,
            48,
            256,
            250,
            26,
            "Microphone",
            crate::theme::current().text,
            17,
            600,
        );
        let mic_label = if s.hardware_mic_muted == Some(true) {
            "Muted on headset"
        } else if effective {
            "Muted"
        } else {
            ""
        };
        text(
            dc,
            48,
            283,
            270,
            20,
            mic_label,
            if effective {
                crate::theme::current().accent
            } else {
                crate::theme::current().muted
            },
            12,
            400,
        );
        round(
            dc,
            rect(363, 283, 129, 28),
            if focus == 0 {
                crate::theme::current().accent
            } else {
                crate::theme::current().surface
            },
            12,
        );
        text(
            dc,
            364,
            284,
            106,
            26,
            if s.audio.microphone_muted {
                "Unmute mic"
            } else {
                "Mute mic"
            },
            crate::theme::current().text,
            13,
            600,
        );
        value(
            dc,
            387,
            256,
            105,
            25,
            &percent(mic.or(s.audio.microphone)),
            crate::theme::current().accent,
            14,
            600,
        );
        slider(dc, 318, mic.or(s.audio.microphone), focus == 1);
        text(
            dc,
            48,
            337,
            400,
            20,
            "Mic preset",
            crate::theme::current().muted,
            11,
            600,
        );
        for (i, name) in ["Default", "Mic boost", "Broadcast", "Conference"]
            .iter()
            .enumerate()
        {
            let active = s.mic_eq == Some(32 + i as u8);
            round(
                dc,
                rect(48 + i as i32 * 112, 362, 108, 30),
                if active {
                    crate::theme::current().accent
                } else {
                    crate::theme::current().surface
                },
                8,
            );
            text(
                dc,
                55 + i as i32 * 112,
                364,
                96,
                26,
                name,
                crate::theme::current().text,
                12,
                if active { 600 } else { 400 },
            );
        }
        text(
            dc,
            48,
            399,
            280,
            24,
            "Sidetone · hear your own voice",
            crate::theme::current().muted,
            12,
            400,
        );
        value(
            dc,
            416,
            399,
            76,
            24,
            &s.sidetone
                .map(|v| format!("{v} / 15"))
                .unwrap_or_else(|| "— / 15".into()),
            crate::theme::current().accent,
            12,
            600,
        );
        slider(
            dc,
            436,
            s.sidetone.map(|v| ((v.min(15) as u16 * 100) / 15) as u8),
            false,
        );
        if focus == 4 {
            text(
                dc,
                350,
                337,
                142,
                20,
                "← / → select",
                crate::theme::current().accent,
                11,
                400,
            );
        }
        if focus == 5 {
            text(
                dc,
                340,
                399,
                70,
                24,
                "← / →",
                crate::theme::current().accent,
                11,
                400,
            );
        }
        card(dc, rect(30, 460, 480, 74), 20);
        text(
            dc,
            48,
            466,
            250,
            27,
            "Game volume",
            crate::theme::current().text,
            17,
            600,
        );
        value(
            dc,
            387,
            466,
            105,
            25,
            &format!(
                "{}{}",
                percent(game.or(s.audio.speaker)),
                if s.audio.speaker_muted {
                    " · muted"
                } else {
                    ""
                }
            ),
            crate::theme::current().accent,
            14,
            600,
        );
        slider(dc, 508, game.or(s.audio.speaker), focus == 2);
        card(dc, rect(30, 546, 480, 116), 20);
        text(
            dc,
            48,
            554,
            290,
            26,
            "Game / Chat balance",
            crate::theme::current().text,
            17,
            600,
        );
        let balance = balance.or(s.game_chat_balance).map(|v| v.min(20));
        let available = balance.is_some();
        round(
            dc,
            rect(410, 556, 82, 28),
            if focus == 7 {
                crate::theme::current().accent
            } else {
                crate::theme::current().surface
            },
            10,
        );
        text(
            dc,
            425,
            557,
            62,
            26,
            "Center",
            if available {
                crate::theme::current().text
            } else {
                crate::theme::current().muted
            },
            12,
            600,
        );
        let (game_label, chat_label) = balance
            .map(|v| {
                let (game, chat) = crate::mixer::levels(v);
                (format!("Game {game}%"), format!("Chat {chat}%"))
            })
            .unwrap_or_else(|| ("Game --".into(), "Chat --".into()));
        text(
            dc,
            48,
            588,
            155,
            22,
            &game_label,
            crate::theme::current().accent,
            13,
            600,
        );
        text(
            dc,
            219,
            588,
            130,
            22,
            if balance == Some(10) {
                "Balanced"
            } else if available {
                "Custom balance"
            } else {
                "Unavailable"
            },
            crate::theme::current().muted,
            11,
            400,
        );
        value(
            dc,
            395,
            588,
            97,
            22,
            &chat_label,
            crate::theme::current().accent,
            13,
            600,
        );
        round(
            dc,
            rect(48, 623, 444, 6),
            crate::theme::current().surface,
            6,
        );
        fill(dc, rect(269, 618, 2, 16), crate::theme::current().muted);
        if let Some(v) = balance {
            let x = 48 + 444 * v as i32 / 20;
            round(
                dc,
                rect(x.min(270), 623, (x - 270).abs().max(2), 6),
                crate::theme::current().accent,
                6,
            );
            round(
                dc,
                rect(x - 7, 619, 14, 14),
                crate::theme::current().text,
                14,
            );
        }
        text(
            dc,
            48,
            644,
            444,
            18,
            if focus == 6 {
                "Arrows adjust - Home / End - Center restores balance"
            } else {
                "Route game audio to Game; voice apps to Chat."
            },
            if focus == 6 {
                crate::theme::current().accent
            } else {
                crate::theme::current().muted
            },
            11,
            400,
        );
        card(dc, rect(30, 674, 480, 55), 18);
        text(
            dc,
            48,
            677,
            315,
            25,
            "Headset mute notifications",
            crate::theme::current().text,
            14,
            600,
        );
        text(
            dc,
            48,
            702,
            318,
            19,
            "Get an alert when you mute on the headset.",
            crate::theme::current().muted,
            11,
            400,
        );
        round(
            dc,
            rect(432, 688, 58, 27),
            if s.notifications_enabled {
                crate::theme::current().accent
            } else {
                crate::theme::current().surface
            },
            27,
        );
        round(
            dc,
            rect(if s.notifications_enabled { 464 } else { 436 }, 692, 19, 19),
            crate::theme::current().bg,
            19,
        );
        if focus == 3 {
            text(
                dc,
                398,
                689,
                30,
                23,
                "›",
                crate::theme::current().accent,
                18,
                600,
            );
        }
        let footer = s.last_error.clone().unwrap_or_else(|| {
            s.battery_age_seconds
                .map(|a| format!("Battery checked {a}s ago  ·  Close to keep running in tray"))
                .unwrap_or_else(|| "Waiting for device  ·  Close to keep running in tray".into())
        });
        text(
            dc,
            30,
            729,
            480,
            25,
            &footer,
            if s.last_error.is_some() {
                crate::theme::current().accent
            } else {
                crate::theme::current().muted
            },
            11,
            400,
        );
    }
}
enum FeatureCommand {
    Hardware(Action),
    Preset(usize),
    Adjust(usize, i8),
    Apply,
    Revert,
}
struct FeatureButton {
    r: RECT,
    label: String,
    command: FeatureCommand,
    enabled: bool,
    selected: bool,
}
fn feature_buttons(s: &State, page: u8) -> Vec<FeatureButton> {
    let mut out = Vec::new();
    let mut add = |x, y, w, label: String, command, enabled, selected| {
        out.push(FeatureButton {
            r: rect(x, y, w, 30),
            label,
            command,
            enabled,
            selected,
        })
    };
    match page {
        1 => {
            for (i, (name, _)) in crate::features::PRESETS.iter().enumerate() {
                add(
                    48 + i as i32 * 112,
                    150,
                    104,
                    (*name).into(),
                    FeatureCommand::Preset(i),
                    s.features.bands.is_some(),
                    DRAFT.lock().unwrap().or(s.features.bands)
                        == Some(crate::features::PRESETS[i].1),
                );
            }
            for i in 0..10 {
                add(
                    358,
                    218 + i as i32 * 35,
                    40,
                    "-".into(),
                    FeatureCommand::Adjust(i, -1),
                    s.features.bands.is_some(),
                    false,
                );
                add(
                    444,
                    218 + i as i32 * 35,
                    40,
                    "+".into(),
                    FeatureCommand::Adjust(i, 1),
                    s.features.bands.is_some(),
                    false,
                );
            }
            add(
                48,
                624,
                210,
                "Revert edits".into(),
                FeatureCommand::Revert,
                s.features.bands.is_some(),
                false,
            );
            add(
                274,
                624,
                210,
                "Apply to current slot".into(),
                FeatureCommand::Apply,
                s.features.bands.is_some(),
                true,
            );
        }
        2 => {
            for (i, value) in [0, 15, 30, 45, 60].iter().enumerate() {
                add(
                    48 + i as i32 * 89,
                    145,
                    81,
                    if *value == 0 {
                        "Never".into()
                    } else {
                        format!("{value} min")
                    },
                    FeatureCommand::Hardware(Action::SetSleep(*value)),
                    s.features.sleep.is_some(),
                    s.features.sleep == Some(*value),
                );
            }
            for (i, (name, value)) in [("Off", 0), ("ANC", 1), ("Ambient", 80)].iter().enumerate() {
                add(
                    48 + i as i32 * 150,
                    247,
                    145,
                    (*name).into(),
                    FeatureCommand::Hardware(Action::SetAnc([
                        *value,
                        s.features.anc.map(|v| v[1]).unwrap_or(1),
                    ])),
                    s.features.anc.is_some(),
                    s.features.anc.is_some_and(|v| v[0] == *value),
                );
            }
            for (i, (name, value)) in [("Range", 0), ("Low latency", 1)].iter().enumerate() {
                add(
                    48 + i as i32 * 225,
                    349,
                    215,
                    (*name).into(),
                    FeatureCommand::Hardware(Action::SetWireless(*value)),
                    s.features.wireless.is_some(),
                    s.features.wireless == Some(*value),
                );
            }
            for (i, value) in [false, true].iter().enumerate() {
                add(
                    48 + i as i32 * 225,
                    451,
                    215,
                    if *value { "Enabled" } else { "Disabled" }.into(),
                    FeatureCommand::Hardware(Action::SetVoicePrompt(*value)),
                    s.features.voice_prompt.is_some(),
                    s.features.voice_prompt == Some(*value),
                );
            }
            for (i, name) in ["Connection", "Battery level", "Low battery"]
                .iter()
                .enumerate()
            {
                add(
                    48 + i as i32 * 150,
                    553,
                    145,
                    (*name).into(),
                    FeatureCommand::Hardware(Action::SetLed(i as u8)),
                    s.features.led.is_some(),
                    s.features.led == Some(i as u8),
                );
            }
        }
        3 => {
            for (i, name) in crate::theme::NAMES.iter().enumerate() {
                add(
                    340,
                    128 + i as i32 * 126,
                    144,
                    if s.theme as usize == i {
                        "Selected".into()
                    } else {
                        format!("Use {name}")
                    },
                    FeatureCommand::Hardware(Action::SetTheme(i as u8)),
                    true,
                    s.theme as usize == i,
                );
            }
        }
        _ => {}
    }
    out
}
fn edit_eq_at(index: usize, x: i32) {
    let s = state();
    let mut draft = DRAFT.lock().unwrap();
    if let Some(mut bands) = draft.or(s.features.bands) {
        bands[index] = (((x - 128).clamp(0, 200) * 24 + 100) / 200 - 12) as i8;
        *draft = Some(bands);
    }
}
fn activate_feature(command: FeatureCommand) {
    let s = state();
    match command {
        FeatureCommand::Hardware(action) => send(action),
        FeatureCommand::Preset(index) => {
            *DRAFT.lock().unwrap() = Some(crate::features::PRESETS[index].1)
        }
        FeatureCommand::Adjust(index, delta) => {
            let mut draft = DRAFT.lock().unwrap();
            if let Some(mut bands) = draft.or(s.features.bands) {
                bands[index] = (bands[index] + delta).clamp(-12, 12);
                *draft = Some(bands);
            }
        }
        FeatureCommand::Apply => {
            if let Some(bands) = *DRAFT.lock().unwrap() {
                send(Action::SetSpeakerBands(bands));
            }
        }
        FeatureCommand::Revert => *DRAFT.lock().unwrap() = None,
    }
}
unsafe fn paint_features(dc: HDC, s: &State, page: u8) {
    unsafe {
        let p = crate::theme::current();
        match page {
            1 => {
                card(dc, rect(30, 90, 480, 610), 20);
                text(dc, 48, 103, 440, 28, "Speaker equalizer", p.text, 21, 600);
                text(
                    dc,
                    48,
                    132,
                    440,
                    18,
                    "Scythe presets / 10 bands / -12 to +12 dB",
                    p.muted,
                    12,
                    400,
                );
                text(
                    dc,
                    48,
                    188,
                    440,
                    22,
                    "FREQUENCY                        GAIN",
                    p.muted,
                    11,
                    600,
                );
                let bands = DRAFT.lock().unwrap().or(s.features.bands);
                for (i, name) in crate::features::FREQUENCIES.iter().enumerate() {
                    let y = 218 + i as i32 * 35;
                    text(
                        dc,
                        48,
                        y + 4,
                        80,
                        24,
                        &format!("{name} Hz"),
                        p.text,
                        14,
                        400,
                    );
                    let db = bands.map(|v| v[i]);
                    round(dc, rect(128, y + 13, 200, 5), p.surface, 4);
                    if let Some(v) = db {
                        round(
                            dc,
                            rect(128 + (v as i32 + 12) * 200 / 24 - 4, y + 8, 10, 15),
                            p.accent,
                            8,
                        );
                    }
                    text(
                        dc,
                        403,
                        y + 3,
                        38,
                        25,
                        &db.map(|v| format!("{v:+}")).unwrap_or("--".into()),
                        p.text,
                        14,
                        600,
                    );
                }
                text(
                    dc,
                    48,
                    578,
                    440,
                    21,
                    &s.preset
                        .map(|v| format!("Editing headset slot {v}; other slots are preserved."))
                        .unwrap_or("Connect the headset to read its current slot.".into()),
                    p.muted,
                    12,
                    400,
                );
                text(
                    dc,
                    48,
                    661,
                    440,
                    24,
                    "Presets stage edits. Apply writes the displayed bands.",
                    p.muted,
                    12,
                    400,
                );
            }
            2 => {
                for (y, title, detail) in [
                    (
                        90,
                        "Auto sleep",
                        "Power off after silence; Never disables the timer.",
                    ),
                    (
                        192,
                        "Noise cancellation",
                        "Switch modes while preserving the current ANC strength.",
                    ),
                    (
                        294,
                        "Wireless mode",
                        "Choose maximum range or lower wireless latency.",
                    ),
                    (
                        396,
                        "Mute voice prompt",
                        "Spoken headset microphone mute / unmute feedback.",
                    ),
                    (
                        498,
                        "Dongle LED",
                        "Choose what the dongle indicator reports.",
                    ),
                ] {
                    card(dc, rect(30, y, 480, 92), 18);
                    text(dc, 48, y + 9, 440, 25, title, p.text, 17, 600);
                    text(dc, 48, y + 34, 440, 19, detail, p.muted, 12, 400);
                }
                card(dc, rect(30, 604, 480, 96), 18);
                text(
                    dc,
                    48,
                    615,
                    440,
                    24,
                    "Microphone enhancements",
                    p.text,
                    17,
                    600,
                );
                text(
                    dc,
                    48,
                    644,
                    440,
                    20,
                    "Normalization, clarity, and noise reduction.",
                    p.muted,
                    12,
                    400,
                );
                text(
                    dc,
                    48,
                    666,
                    440,
                    20,
                    "Not available in this version.",
                    p.muted,
                    12,
                    400,
                );
            }
            3 => {
                for (i, name) in crate::theme::NAMES.iter().enumerate() {
                    let y = 90 + i as i32 * 126;
                    card(dc, rect(30, y, 480, 112), 18);
                    text(dc, 48, y + 12, 280, 28, name, p.text, 20, 600);
                    for (j, color) in crate::theme::SWATCHES[i].iter().enumerate() {
                        round(
                            dc,
                            rect(48 + j as i32 * 52, y + 52, 42, 34),
                            crate::theme::swatch(*color),
                            10,
                        );
                    }
                }
                text(
                    dc,
                    48,
                    615,
                    440,
                    24,
                    "Themes apply to the dashboard and tray preview.",
                    p.muted,
                    13,
                    400,
                );
                text(
                    dc,
                    48,
                    643,
                    440,
                    24,
                    "Battery glow keeps its green / yellow / red thresholds.",
                    p.muted,
                    12,
                    400,
                );
            }
            _ => {}
        }
        for (index, button) in feature_buttons(s, page).iter().enumerate() {
            let focused = FEATURE_FOCUS.load(Ordering::Relaxed) == index;
            if focused && button.enabled {
                round(
                    dc,
                    rect(
                        button.r.left - 2,
                        button.r.top - 2,
                        button.r.right - button.r.left + 4,
                        34,
                    ),
                    p.text,
                    11,
                );
            }
            round(
                dc,
                button.r,
                if button.selected { p.accent } else { p.bg },
                9,
            );
            text(
                dc,
                button.r.left + 8,
                button.r.top + 4,
                button.r.right - button.r.left - 12,
                24,
                &button.label,
                if !button.enabled {
                    p.surface
                } else if button.selected {
                    crate::theme::on_accent()
                } else {
                    p.text
                },
                12,
                600,
            );
        }
        text(
            dc,
            30,
            712,
            480,
            21,
            s.last_error
                .as_deref()
                .unwrap_or("F1-F4: pages   /   Tab: next control   /   Enter: select"),
            if s.last_error.is_some() {
                p.accent
            } else {
                p.muted
            },
            11,
            400,
        );
    }
}

unsafe fn notify(hwnd: HWND, action: u32, balloon: bool) {
    unsafe {
        let mut n: NOTIFYICONDATAW = zeroed();
        n.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        n.hWnd = hwnd;
        n.uID = 1;
        n.uFlags = NIF_ICON | NIF_MESSAGE;
        n.uCallbackMessage = TRAY;
        n.hIcon = ui(hwnd).icon;
        if balloon {
            n.uFlags = NIF_INFO;
            n.dwInfoFlags = NIIF_INFO;
            let title = wide("Scythe - Microphone muted");
            n.szInfoTitle[..title.len()].copy_from_slice(&title);
            let body = wide("Muted on headset. Press the headset button to unmute.");
            n.szInfo[..body.len()].copy_from_slice(&body);
        }
        // No NIF_TIP: only our strictly bounded preview should appear on hover.
        Shell_NotifyIconW(action, &n);
    }
}
unsafe fn tray_rect(hwnd: HWND) -> Option<RECT> {
    unsafe {
        let mut id: NOTIFYICONIDENTIFIER = zeroed();
        id.cbSize = size_of::<NOTIFYICONIDENTIFIER>() as u32;
        id.hWnd = hwnd;
        id.uID = 1;
        let mut r = zeroed();
        if Shell_NotifyIconGetRect(&id, &mut r) >= 0 {
            Some(r)
        } else {
            None
        }
    }
}
// Borderless client and window dimensions are identical.
unsafe fn fit_dashboard(w: i32, h: i32, work: RECT, _dpi: u32) -> (i32, i32) {
    let aw = (work.right - work.left).max(1);
    let ah = (work.bottom - work.top).max(1);
    let fit = (aw as f64 / w.max(1) as f64)
        .min(ah as f64 / h.max(1) as f64)
        .min(1.0);
    ((w as f64 * fit) as i32, (h as f64 * fit) as i32)
}
unsafe fn place(hwnd: HWND, anchor: RECT, w: i32, h: i32, activate: bool) {
    unsafe {
        let monitor = MonitorFromRect(&anchor, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut info);
        let r = info.rcWork;
        let (w, h) = if activate {
            fit_dashboard(w, h, r, ui(hwnd).dpi)
        } else {
            (w, h)
        };
        let x = (anchor.right - w).clamp(r.left, (r.right - w).max(r.left));
        let preferred = if anchor.top - h - 10 >= r.top {
            anchor.top - h - 10
        } else {
            anchor.bottom + 10
        };
        let y = preferred.clamp(r.top, (r.bottom - h).max(r.top));
        SetWindowPos(
            hwnd,
            if activate { HWND_TOP } else { HWND_TOPMOST },
            x,
            y,
            w,
            h,
            if activate { 0 } else { SWP_NOACTIVATE },
        );
        ShowWindow(hwnd, if activate { SW_SHOW } else { SW_SHOWNOACTIVATE });
        if activate {
            SetForegroundWindow(hwnd);
            SetFocus(hwnd);
        }
    }
}
unsafe fn show(hwnd: HWND) {
    unsafe {
        ShowWindow(ui(hwnd).tip, SW_HIDE);
        let anchor = tray_rect(hwnd).unwrap_or_else(|| {
            let mut p = zeroed();
            GetCursorPos(&mut p);
            rect(p.x, p.y, 1, 1)
        });
        let scale = ui(hwnd).dpi as f64 / 96.;
        place(
            hwnd,
            anchor,
            (W as f64 * scale) as i32,
            (H as f64 * scale) as i32,
            true,
        );
        InvalidateRect(hwnd, null(), 0);
    }
}
unsafe fn point(hwnd: HWND, l: LPARAM) -> (i32, i32) {
    unsafe {
        let mut r = zeroed();
        GetClientRect(hwnd, &mut r);
        (
            (l as i16 as i32) * W / (r.right.max(1)),
            ((l >> 16) as i16 as i32) * H / (r.bottom.max(1)),
        )
    }
}
unsafe fn volume(hwnd: HWND, x: i32) {
    unsafe {
        let u = ui(hwnd);
        let v = ((x - 48) * 100 / 444).clamp(0, 100) as u8;
        if u.drag == 1 {
            u.mic = Some(v);
            send(Action::SetMicVolume(v));
        } else if u.drag == 2 {
            u.game = Some(v);
            send(Action::SetSpeakerVolume(v));
        }
        if u.drag == 6 {
            let raw = (((x - 48) * 20 + 222) / 444).clamp(0, 20) as u8;
            u.balance = Some(raw);
            send(Action::SetGameChatBalance(raw));
        }
        if u.drag == 5 {
            send(Action::SetSidetone(((x - 48) * 15 / 444).clamp(0, 15) as u8));
        }
        InvalidateRect(hwnd, null(), 0);
    }
}
unsafe extern "system" fn window(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            return 1;
        }
        if GetWindowLongPtrW(hwnd, GWLP_USERDATA) == 0 {
            return DefWindowProcW(hwnd, msg, w, l);
        }
        if msg == ui(hwnd).taskbar_created {
            notify(hwnd, NIM_ADD, false);
            return 0;
        }
        match msg {
            TRAY => {
                match l as u32 {
                    WM_MOUSEMOVE => {
                        if IsWindowVisible(hwnd) == 0
                            && let Some(r) = tray_rect(hwnd)
                        {
                            let mut p = zeroed();
                            GetCursorPos(&mut p);
                            if PtInRect(&r, p) != 0 {
                                place(ui(hwnd).tip, r, 290, 100, false);
                            }
                        }
                    }
                    WM_LBUTTONUP => show(hwnd),
                    WM_RBUTTONUP => {
                        ShowWindow(ui(hwnd).tip, SW_HIDE);
                        let menu = CreatePopupMenu();
                        AppendMenuW(menu, MF_STRING, 1, wide("Open Scythe").as_ptr());
                        AppendMenuW(menu, MF_STRING, 2, wide("Refresh status").as_ptr());
                        AppendMenuW(menu, MF_SEPARATOR, 0, null());
                        AppendMenuW(menu, MF_STRING, 3, wide("Exit").as_ptr());
                        let mut p = zeroed();
                        GetCursorPos(&mut p);
                        SetForegroundWindow(hwnd);
                        let command = TrackPopupMenu(
                            menu,
                            TPM_RETURNCMD | TPM_NONOTIFY,
                            p.x,
                            p.y,
                            0,
                            hwnd,
                            null(),
                        );
                        DestroyMenu(menu);
                        match command {
                            1 => show(hwnd),
                            2 => REFRESH.get().unwrap().store(true, Ordering::Relaxed),
                            3 => {
                                DestroyWindow(hwnd);
                            }
                            _ => {}
                        }
                        PostMessageW(hwnd, WM_NULL, 0, 0);
                    }
                    _ => {}
                }
                0
            }
            WM_TIMER => {
                if w == 1 {
                    let mut p = zeroed();
                    GetCursorPos(&mut p);
                    if !tray_rect(hwnd).is_some_and(|r| PtInRect(&r, p) != 0) {
                        ShowWindow(ui(hwnd).tip, SW_HIDE);
                    }
                } else {
                    let s = state();
                    let u = ui(hwnd);
                    if let Some(previous) = u.baseline
                        && previous != s.mic_mute_notifications
                        && s.notifications_enabled
                    {
                        notify(hwnd, NIM_MODIFY, true);
                    }
                    ui(hwnd).baseline = Some(s.mic_mute_notifications);
                    if IsWindowVisible(hwnd) != 0 {
                        InvalidateRect(hwnd, null(), 0);
                    }
                    if IsWindowVisible(ui(hwnd).tip) != 0 {
                        InvalidateRect(ui(hwnd).tip, null(), 0);
                    }
                }
                0
            }
            WM_NCHITTEST => {
                let mut p = POINT {
                    x: l as i16 as i32,
                    y: (l >> 16) as i16 as i32,
                };
                ScreenToClient(hwnd, &mut p);
                let mut r = zeroed();
                GetClientRect(hwnd, &mut r);
                let x = p.x * W / r.right.max(1);
                let y = p.y * H / r.bottom.max(1);
                let button =
                    (14..44).contains(&y) && ((426..464).contains(&x) || (472..510).contains(&x));
                if (0..44).contains(&y) && !button {
                    HTCAPTION as isize
                } else {
                    HTCLIENT as isize
                }
            }
            WM_LBUTTONDOWN => {
                let (x, y) = point(hwnd, l);
                let s = state();
                if (48..73).contains(&y) && (30..510).contains(&x) {
                    PAGE.store(((x - 30) / 122).min(3) as u8, Ordering::Relaxed);
                    FEATURE_FOCUS.store(0, Ordering::Relaxed);
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                if PAGE.load(Ordering::Relaxed) == 1
                    && (128..=328).contains(&x)
                    && (218..568).contains(&y)
                    && (y - 218) % 35 < 30
                    && s.features.bands.is_some()
                {
                    let index = ((y - 218) / 35) as usize;
                    ui(hwnd).drag = 100 + index as i32;
                    SetCapture(hwnd);
                    edit_eq_at(index, x);
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                if PAGE.load(Ordering::Relaxed) > 0 && y >= 82 {
                    let buttons = feature_buttons(&s, PAGE.load(Ordering::Relaxed));
                    for (index, button) in buttons.into_iter().enumerate() {
                        if x >= button.r.left
                            && x < button.r.right
                            && y >= button.r.top
                            && y < button.r.bottom
                        {
                            FEATURE_FOCUS.store(index, Ordering::Relaxed);
                            if button.enabled {
                                activate_feature(button.command);
                            }
                            break;
                        }
                    }
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                if (14..44).contains(&y) && ((426..464).contains(&x) || (472..510).contains(&x)) {
                    ui(hwnd).focus = if x < 464 { 8 } else { 9 };
                    ShowWindow(hwnd, SW_HIDE);
                } else if (363..492).contains(&x) && (283..311).contains(&y) {
                    ui(hwnd).focus = 0;
                    send(Action::SetMicMute(!s.audio.microphone_muted));
                } else if (432..490).contains(&x) && (688..715).contains(&y) {
                    ui(hwnd).focus = 3;
                    send(Action::SetMicNotifications(!s.notifications_enabled));
                } else if (410..492).contains(&x) && (556..584).contains(&y) {
                    ui(hwnd).focus = 7;
                    if s.game_chat_balance.is_some() {
                        send(Action::SetGameChatBalance(10));
                    }
                } else if (48..492).contains(&x) && (362..392).contains(&y) && (x - 48) % 112 < 108
                {
                    ui(hwnd).focus = 4;
                    send(Action::SetMicPreset(32 + ((x - 48) / 112).min(3) as u8));
                } else if (40..500).contains(&x) {
                    let which = if (307..335).contains(&y) {
                        1
                    } else if (425..452).contains(&y) {
                        5
                    } else if (612..639).contains(&y) {
                        6
                    } else if (497..527).contains(&y) {
                        2
                    } else {
                        -1
                    };
                    if which > 0 {
                        let available = if which == 1 {
                            s.audio.microphone.is_some()
                        } else if which == 6 {
                            s.game_chat_balance.is_some()
                        } else if which == 5 {
                            s.sidetone.is_some()
                        } else {
                            s.audio.speaker.is_some()
                        };
                        if available {
                            ui(hwnd).drag = which;
                            ui(hwnd).focus = which;
                            SetCapture(hwnd);
                            volume(hwnd, x);
                        }
                    }
                }
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_MOUSEMOVE => {
                if ui(hwnd).drag >= 100 {
                    edit_eq_at((ui(hwnd).drag - 100) as usize, point(hwnd, l).0);
                    InvalidateRect(hwnd, null(), 0);
                } else if ui(hwnd).drag > 0 {
                    volume(hwnd, point(hwnd, l).0);
                }
                0
            }
            WM_LBUTTONUP | WM_CAPTURECHANGED => {
                if ui(hwnd).drag > 0 {
                    ui(hwnd).drag = 0;
                    ui(hwnd).mic = None;
                    ui(hwnd).game = None;
                    ui(hwnd).balance = None;
                    ReleaseCapture();
                }
                0
            }
            WM_KEYDOWN => {
                let s = state();
                if (0x70..=0x73).contains(&w) {
                    PAGE.store((w - 0x70) as u8, Ordering::Relaxed);
                    FEATURE_FOCUS.store(0, Ordering::Relaxed);
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                if PAGE.load(Ordering::Relaxed) > 0 && w != 0x1b {
                    let buttons = feature_buttons(&s, PAGE.load(Ordering::Relaxed));
                    let index = FEATURE_FOCUS
                        .load(Ordering::Relaxed)
                        .min(buttons.len().saturating_sub(1));
                    if !buttons.is_empty() {
                        if w == 9 {
                            FEATURE_FOCUS.store((index + 1) % buttons.len(), Ordering::Relaxed);
                        }
                        if w == 0x20 || w == 0x0d {
                            let button = buttons.into_iter().nth(index).unwrap();
                            if button.enabled {
                                activate_feature(button.command);
                            }
                        }
                    }
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                match w {
                    0x1b => {
                        ShowWindow(hwnd, SW_HIDE);
                    }
                    0x09 => {
                        ui(hwnd).focus = match ui(hwnd).focus {
                            0 => 1,
                            1 => 4,
                            4 => 5,
                            5 => 2,
                            2 => 6,
                            6 => 7,
                            7 => 3,
                            3 => 8,
                            8 => 9,
                            _ => 0,
                        };
                    }
                    0x20 | 0x0d => {
                        if matches!(ui(hwnd).focus, 8 | 9) {
                            ShowWindow(hwnd, SW_HIDE);
                        }
                        if ui(hwnd).focus == 0 {
                            send(Action::SetMicMute(!s.audio.microphone_muted));
                        }
                        if ui(hwnd).focus == 7 && s.game_chat_balance.is_some() {
                            send(Action::SetGameChatBalance(10));
                        }
                        if ui(hwnd).focus == 3 {
                            send(Action::SetMicNotifications(!s.notifications_enabled));
                        }
                    }
                    0x24 | 0x23 if ui(hwnd).focus == 6 => {
                        if s.game_chat_balance.is_some() {
                            send(Action::SetGameChatBalance(if w == 0x24 { 0 } else { 20 }));
                        }
                    }
                    0x25 | 0x27 => {
                        let delta: i32 = if w == 0x25 { -2 } else { 2 };
                        match ui(hwnd).focus {
                            6 => {
                                if let Some(v) = s.game_chat_balance {
                                    send(Action::SetGameChatBalance(
                                        (v as i32 + delta.signum()).clamp(0, 20) as u8,
                                    ));
                                }
                            }
                            4 => {
                                let v = s.mic_eq.unwrap_or(32) as i32;
                                send(Action::SetMicPreset(
                                    (v + delta.signum()).clamp(32, 35) as u8
                                ));
                            }
                            5 => {
                                if let Some(v) = s.sidetone {
                                    send(Action::SetSidetone(
                                        (v as i32 + delta.signum()).clamp(0, 15) as u8,
                                    ));
                                }
                            }
                            1 => {
                                if let Some(v) = s.audio.microphone {
                                    send(Action::SetMicVolume(
                                        (v as i32 + delta).clamp(0, 100) as u8
                                    ));
                                }
                            }
                            2 => {
                                if let Some(v) = s.audio.speaker {
                                    send(Action::SetSpeakerVolume(
                                        (v as i32 + delta).clamp(0, 100) as u8,
                                    ));
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_PAINT => {
                let mut ps = zeroed();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut r = zeroed();
                GetClientRect(hwnd, &mut r);
                let mem = CreateCompatibleDC(dc);
                let bm = CreateCompatibleBitmap(dc, r.right.max(1), r.bottom.max(1));
                let old = SelectObject(mem, bm);
                let target = crate::render::target(mem, r.right.max(1), r.bottom.max(1), W, H);
                let u = ui(hwnd);
                paint(mem, &state(), false, u.focus, u.mic, u.game, u.balance);
                drop(target);
                BitBlt(dc, 0, 0, r.right, r.bottom, mem, 0, 0, SRCCOPY);
                SelectObject(mem, old);
                DeleteObject(bm);
                DeleteDC(mem);
                EndPaint(hwnd, &ps);
                0
            }
            WM_ERASEBKGND => 1,
            WM_CLOSE => {
                ShowWindow(hwnd, SW_HIDE);
                0
            }
            WM_SIZE if w == SIZE_MINIMIZED as usize => {
                ShowWindow(hwnd, SW_HIDE);
                0
            }
            WM_DPICHANGED => {
                ui(hwnd).dpi = (w & 0xffff) as u32;
                let r = &*(l as *const RECT);
                let mut info: MONITORINFO = zeroed();
                info.cbSize = size_of::<MONITORINFO>() as u32;
                GetMonitorInfoW(MonitorFromRect(r, MONITOR_DEFAULTTONEAREST), &mut info);
                let work = info.rcWork;
                let (width, height) =
                    fit_dashboard(r.right - r.left, r.bottom - r.top, work, ui(hwnd).dpi);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    r.left.clamp(work.left, (work.right - width).max(work.left)),
                    r.top.clamp(work.top, (work.bottom - height).max(work.top)),
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                InvalidateRect(hwnd, null(), 0);
                0
            }
            SHOW => {
                show(hwnd);
                0
            }
            WM_DESTROY => {
                notify(hwnd, NIM_DELETE, false);
                let data = Box::from_raw(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui);
                DestroyWindow(data.tip);
                DestroyIcon(data.icon);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
unsafe extern "system" fn tooltip(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
            WM_NCHITTEST => HTTRANSPARENT as isize,
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                let mut ps = zeroed();
                let dc = BeginPaint(hwnd, &mut ps);
                let mem = CreateCompatibleDC(dc);
                let mut r = zeroed();
                GetClientRect(hwnd, &mut r);
                let bm = CreateCompatibleBitmap(dc, r.right.max(1), r.bottom.max(1));
                let old = SelectObject(mem, bm);
                let target = crate::render::target(mem, r.right.max(1), r.bottom.max(1), 290, 100);
                paint(mem, &state(), true, -1, None, None, None);
                drop(target);
                BitBlt(dc, 0, 0, r.right, r.bottom, mem, 0, 0, SRCCOPY);
                SelectObject(mem, old);
                DeleteObject(bm);
                DeleteDC(mem);
                EndPaint(hwnd, &ps);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
unsafe fn make_icon() -> HICON {
    unsafe {
        let dc = GetDC(null_mut());
        let mem = CreateCompatibleDC(dc);
        let bm = CreateCompatibleBitmap(dc, 64, 64);
        let old = SelectObject(mem, bm);
        let target = crate::render::target(mem, 64, 64, 32, 32);
        fill(mem, rect(0, 0, 32, 32), crate::theme::current().bg);
        // A broad curved blade and diagonal handle stay legible at tray size.
        let blade = [
            POINT { x: 4, y: 12 },
            POINT { x: 9, y: 5 },
            POINT { x: 18, y: 3 },
            POINT { x: 27, y: 7 },
            POINT { x: 30, y: 14 },
            POINT { x: 23, y: 10 },
            POINT { x: 15, y: 9 },
            POINT { x: 8, y: 12 },
        ];
        crate::render::polygon(mem, &blade, crate::theme::current().accent);
        let handle = [
            POINT { x: 21, y: 9 },
            POINT { x: 24, y: 11 },
            POINT { x: 11, y: 30 },
            POINT { x: 8, y: 28 },
        ];
        crate::render::polygon(mem, &handle, crate::theme::current().text);
        drop(target);
        SelectObject(mem, old);
        DeleteDC(mem);
        ReleaseDC(null_mut(), dc);
        let bits = [0u8; 512];
        let mask = CreateBitmap(64, 64, 1, 1, bits.as_ptr().cast());
        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: bm,
        };
        let icon = CreateIconIndirect(&info);
        DeleteObject(mask);
        DeleteObject(bm);
        icon
    }
}
// Offline export uses exactly the application's painter and the supplied State.
// --render [path.bmp] performs no screen capture or simulated input.
unsafe fn render(path: &std::path::Path, compact: bool, scale: i32) -> std::io::Result<()> {
    unsafe {
        let (width, height) = if compact { (290i32, 100i32) } else { (W, H) };
        let (logical_w, logical_h) = (width, height);
        let (width, height) = (width * scale, height * scale);
        let dc = CreateCompatibleDC(null_mut());
        let (bm, bits) = dib(dc, width, height);
        if bm.is_null() {
            DeleteDC(dc);
            return Err(std::io::Error::last_os_error());
        }
        let old = SelectObject(dc, bm);
        let target = crate::render::target(dc, width, height, logical_w, logical_h);
        paint(dc, &state(), compact, -1, None, None, None);
        drop(target);
        GdiFlush();
        let pixels = std::slice::from_raw_parts(bits, (width * height * 4) as usize);
        let mut data = Vec::with_capacity(54 + pixels.len());
        data.extend_from_slice(b"BM");
        data.extend_from_slice(&(54u32 + pixels.len() as u32).to_le_bytes());
        data.extend_from_slice(&[0; 4]);
        data.extend_from_slice(&54u32.to_le_bytes());
        data.extend_from_slice(&40u32.to_le_bytes());
        data.extend_from_slice(&width.to_le_bytes());
        data.extend_from_slice(&(-height).to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&32u16.to_le_bytes());
        data.extend_from_slice(&[0; 24]);
        data.extend_from_slice(pixels);
        let result = std::fs::write(path, data);
        SelectObject(dc, old);
        DeleteObject(bm);
        DeleteDC(dc);
        result
    }
}
pub fn run(state: Arc<Mutex<State>>, refresh: Arc<AtomicBool>, commands: Sender<Action>) {
    let args: Vec<String> = std::env::args().collect();
    if let Some(index) = args.iter().position(|a| a == "--page")
        && let Some(page) = args.get(index + 1).and_then(|v| v.parse::<u8>().ok())
    {
        PAGE.store(page.min(3), Ordering::Relaxed);
    }
    if let Some(index) = args.iter().position(|a| a == "--theme")
        && let Some(theme) = args.get(index + 1).and_then(|v| v.parse::<u8>().ok())
    {
        state.lock().unwrap().theme = theme.min(3);
    }
    STATE.set(state).ok();
    REFRESH.set(refresh).ok();
    COMMANDS.set(commands).ok();
    unsafe {
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) == 0 {
            SetProcessDPIAware();
        }
        let args: Vec<String> = std::env::args().collect();
        if let Some(i) = args
            .iter()
            .position(|a| a == "--render" || a == "--render-hover" || a == "--render-2x")
        {
            let path = args
                .get(i + 1)
                .filter(|s| !s.starts_with("--"))
                .map(String::as_str)
                .unwrap_or("scythe-ui.bmp");
            if let Err(e) = render(
                std::path::Path::new(path),
                args[i] == "--render-hover",
                if args
                    .iter()
                    .any(|a| a == "--render-scale2" || a == "--render-2x")
                {
                    2
                } else {
                    1
                },
            ) {
                eprintln!("UI render failed: {e}");
            }
            return;
        }
        let instance = GetModuleHandleW(null());
        let name = wide("ScytheTray");
        let tip_name = wide("ScytheStatusPreview");
        let mut class: WNDCLASSW = zeroed();
        class.hInstance = instance;
        class.lpszClassName = name.as_ptr();
        class.lpfnWndProc = Some(window);
        class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        RegisterClassW(&class);
        class.lpszClassName = tip_name.as_ptr();
        class.lpfnWndProc = Some(tooltip);
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            name.as_ptr(),
            wide("Scythe — Headset control").as_ptr(),
            WS_POPUP,
            0,
            0,
            W,
            H,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        let dark: i32 = 1;
        let rounded: i32 = 2;
        DwmSetWindowAttribute(hwnd, 20, (&dark as *const i32).cast(), 4);
        DwmSetWindowAttribute(hwnd, 33, (&rounded as *const i32).cast(), 4);
        let tip = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            tip_name.as_ptr(),
            wide("Scythe status preview").as_ptr(),
            WS_POPUP,
            0,
            0,
            290,
            100,
            hwnd,
            null_mut(),
            instance,
            null(),
        );
        let dc = GetDC(hwnd);
        let dpi = GetDeviceCaps(dc, LOGPIXELSX as i32).max(96) as u32;
        ReleaseDC(hwnd, dc);
        let data = Box::new(Ui {
            tip,
            icon: make_icon(),
            baseline: Some(self::state().mic_mute_notifications),
            drag: 0,
            focus: 0,
            mic: None,
            game: None,
            balance: None,
            dpi,
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(data) as isize);
        SendMessageW(
            hwnd,
            WM_SETICON,
            ICON_SMALL as usize,
            ui(hwnd).icon as isize,
        );
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, ui(hwnd).icon as isize);
        ShowWindow(hwnd, SW_HIDE);
        notify(hwnd, NIM_ADD, false);
        SetTimer(hwnd, 1, 30, None);
        SetTimer(hwnd, 2, 75, None);
        if args.iter().any(|a| a == "--preview" || a == "--show") {
            show(hwnd);
        }
        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
