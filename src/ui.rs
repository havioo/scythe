mod about;
mod accessibility;
mod demo;
use crate::{Action, State, device::wide};
use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::Sender,
    },
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::*,
    UI::{HiDpi::*, Shell::*, WindowsAndMessaging::*},
};
static STATE: OnceLock<Arc<Mutex<State>>> = OnceLock::new();
static REFRESH: OnceLock<Arc<AtomicBool>> = OnceLock::new();
static COMMANDS: OnceLock<Sender<Action>> = OnceLock::new();
static GLOW: OnceLock<Mutex<(String, u32, Vec<u8>)>> = OnceLock::new();
static VIEW: OnceLock<Mutex<View>> = OnceLock::new();
static FADE: AtomicU32 = AtomicU32::new(1000);
static CHARGE_EPOCH: OnceLock<Instant> = OnceLock::new();
static PREVIEW_CHARGE_PHASE: OnceLock<f32> = OnceLock::new();
#[link(name = "user32")]
unsafe extern "system" {
    fn SetCapture(hwnd: HWND) -> HWND;
    fn ReleaseCapture() -> i32;
    fn SetFocus(hwnd: HWND) -> HWND;
    fn GetFocus() -> HWND;
    fn GetKeyState(key: i32) -> i16;
    fn OpenClipboard(hwnd: HWND) -> i32;
    fn CloseClipboard() -> i32;
    fn GetClipboardData(format: u32) -> HANDLE;

}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalLock(memory: HANDLE) -> *mut std::ffi::c_void;
    fn GlobalUnlock(memory: HANDLE) -> i32;
    fn GlobalSize(memory: HANDLE) -> usize;
}
#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: HWND,
        attribute: u32,
        value: *const std::ffi::c_void,
        size: u32,
    ) -> i32;
    fn DwmFlush() -> i32;
}
const TRAY: u32 = WM_APP + 1;
const SHOW: u32 = WM_APP + 2;
pub const ICON_STATUS: u32 = WM_APP + 3;
pub const PREVIEW_STATUS: u32 = WM_APP + 4;
pub const ANIMATION_STATUS: u32 = WM_APP + 5;
static PREVIEW_RESULT: AtomicU32 = AtomicU32::new(0);
static FRAME_PENDING: AtomicBool = AtomicBool::new(false);
const ANIMATION_FRAME: u32 = WM_APP + 6;
const W: i32 = 1040;
const H: i32 = 760;
const PAGE_NAMES: [&str; 5] = [
    "Overview",
    "Sound studio",
    "Microphone",
    "Device",
    "Appearance",
];
#[derive(Clone)]
struct View {
    page: u8,
    focus: usize,
    hover: Option<usize>,
    draft: Option<[i8; 10]>,
    draft_slot: Option<u8>,
    eq_pending: Option<(Instant, u8, [i8; 10])>,
    minutes: String,
    editing: bool,
    select_all: bool,
    caret: usize,
    minutes_dirty: bool,
    error: Option<String>,
    transition: Instant,
    overrides: [Option<i32>; 4],
    override_time: Instant,
    os_reduce_motion: bool,
    sidebar: f32,
    sidebar_from: f32,
    sidebar_open: bool,
    sidebar_started: Instant,
    nav_keyboard: bool,
    keyboard_focus: bool,
    menu_open: Option<u8>,
}
impl Default for View {
    fn default() -> Self {
        Self {
            page: 0,
            focus: usize::MAX,
            hover: None,
            draft: None,
            draft_slot: None,
            eq_pending: None,
            minutes: "15".into(),
            editing: false,
            select_all: false,
            caret: 2,
            minutes_dirty: false,
            error: None,
            transition: Instant::now(),
            overrides: [None; 4],
            override_time: Instant::now(),
            os_reduce_motion: false,
            sidebar: 0.0,
            sidebar_from: 0.0,
            sidebar_open: false,
            sidebar_started: Instant::now(),
            nav_keyboard: false,
            keyboard_focus: false,
            menu_open: None,
        }
    }
}
fn view() -> std::sync::MutexGuard<'static, View> {
    VIEW.get_or_init(|| Mutex::new(View::default()))
        .lock()
        .unwrap()
}
fn sidebar_width(v: &View) -> i32 {
    72 + (132.0 * v.sidebar).round() as i32
}
fn content_shift(v: &View) -> i32 {
    (66.0 * (1.0 - v.sidebar)).round() as i32
}

fn sidebar_progress(from: f32, open: bool, elapsed: f32, reduced: bool) -> f32 {
    let target = if open { 1.0 } else { 0.0 };
    if reduced || elapsed >= 0.28 {
        return target;
    }
    let phase = (elapsed / 0.28).clamp(0.0, 1.0);
    let eased = phase.powi(3) * (phase * (phase * 6.0 - 15.0) + 10.0);
    from + (target - from) * eased
}
fn update_sidebar(v: &mut View, open: bool, reduced: bool) {
    if open != v.sidebar_open {
        v.sidebar_from = v.sidebar;
        v.sidebar_open = open;
        v.sidebar_started = Instant::now();
    }
    v.sidebar = sidebar_progress(
        v.sidebar_from,
        open,
        v.sidebar_started.elapsed().as_secs_f32(),
        reduced || v.os_reduce_motion,
    );
}

unsafe fn refresh_motion_preference() {
    unsafe {
        let mut enabled: i32 = 1;
        if SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        ) != 0
        {
            view().os_reduce_motion = enabled == 0;
        }
    }
}

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    width: i32,
    height: i32,
}
impl Surface {
    unsafe fn new(dc: HDC, width: i32, height: i32) -> Self {
        unsafe {
            let mem = CreateCompatibleDC(dc);
            let bitmap = CreateCompatibleBitmap(dc, width, height);
            let previous = SelectObject(mem, bitmap);
            Self {
                dc: mem,
                bitmap,
                previous,
                width,
                height,
            }
        }
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}
struct SidebarFrame {
    surface: Surface,
    shift: i32,
    left: i32,
    page: u8,
    theme: u8,
}
#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
}
unsafe fn animation_clock(hwnd: HWND, enabled: bool) {
    unsafe {
        use windows_sys::Win32::System::Threading::*;
        if enabled {
            if !ui(hwnd).animation_clock {
                ui(hwnd).animation_clock = timeBeginPeriod(1) == 0;
            }
            if ui(hwnd).animation_timer.is_null() {
                CreateTimerQueueTimer(
                    &mut ui(hwnd).animation_timer,
                    null_mut(),
                    Some(animation_tick),
                    hwnd.cast(),
                    0,
                    1,
                    WT_EXECUTEDEFAULT,
                );
            }
        } else {
            if !ui(hwnd).animation_timer.is_null() {
                DeleteTimerQueueTimer(null_mut(), ui(hwnd).animation_timer, INVALID_HANDLE_VALUE);
                ui(hwnd).animation_timer = null_mut();
            }
            if ui(hwnd).animation_clock {
                timeEndPeriod(1);
                ui(hwnd).animation_clock = false;
            }
        }
    }
}
// A bounded posted tick avoids WM_TIMER's 10 ms minimum on high-refresh displays.
// The callback never touches UI state, and the UI thread presents with DwmFlush.
unsafe extern "system" fn animation_tick(context: *mut std::ffi::c_void, _: bool) {
    unsafe {
        if !FRAME_PENDING.swap(true, Ordering::AcqRel)
            && PostMessageW(context.cast(), ANIMATION_FRAME, 0, 0) == 0
        {
            FRAME_PENDING.store(false, Ordering::Release);
        }
    }
}
fn sidebar_animating(v: &View) -> bool {
    v.sidebar != if v.sidebar_open { 1.0 } else { 0.0 }
}
struct Ui {
    tip: HWND,
    icon: HICON,
    icon_color: u32,
    charging_animation: bool,
    baseline: Option<u64>,
    drag: Option<SliderKind>,
    dpi: u32,
    taskbar_created: u32,
    buffer: Option<Surface>,
    sidebar_frame: Option<SidebarFrame>,
    animation_clock: bool,
    animation_timer: HANDLE,
    frame_times: std::collections::VecDeque<u32>,
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
fn contains(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}
fn mix(a: u32, b: u32, t: f32) -> u32 {
    (0..3).fold(0, |out, i| {
        let shift = i * 8;
        let av = ((a >> shift) & 255) as f32;
        let bv = ((b >> shift) & 255) as f32;
        out | (((av + (bv - av) * t).round() as u32) << shift)
    })
}
fn faded(c: u32) -> u32 {
    mix(
        crate::theme::current().bg,
        c,
        FADE.load(Ordering::Relaxed) as f32 / 1000.,
    )
}
unsafe fn fill(dc: HDC, r: RECT, c: u32) {
    unsafe {
        crate::render::fill(dc, r, faded(c));
    }
}
unsafe fn round(dc: HDC, r: RECT, c: u32, d: i32) {
    unsafe {
        crate::render::rounded(dc, r, faded(c), d);
    }
}
unsafe fn card(dc: HDC, r: RECT) {
    unsafe {
        round(dc, r, crate::theme::current().panel, 18);
    }
}
#[allow(clippy::too_many_arguments)]
unsafe fn text(dc: HDC, x: i32, y: i32, w: i32, h: i32, s: &str, c: u32, size: i32, weight: i32) {
    unsafe {
        crate::render::text(dc, rect(x, y, w, h), s, faded(c), size, weight, false);
    }
}
unsafe fn label(dc: HDC, x: i32, y: i32, w: i32, s: &str) {
    unsafe {
        text(dc, x, y, w, 20, s, crate::theme::current().muted, 11, 600);
    }
}
unsafe fn line(dc: HDC, points: &[POINT], c: u32, width: f32) {
    unsafe {
        crate::render::line(dc, points, faded(c), width);
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
unsafe fn percentage(dc: HDC, x: i32, y: i32, value: &str, color: u32, charging: bool) {
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
            crate::render::fill(mem, rect(0, 0, w as i32, h as i32), 0);
            crate::render::text(mem, rect(24, 12, 250, 110), value, 0xffffff, 76, 600, false);
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
            rect(x, y, w as i32, h as i32),
            w as i32,
            h as i32,
            &cached.2,
        );
        SelectObject(mem, old);
        DeleteObject(bm);
        DeleteDC(mem);
        battery_text(
            dc,
            rect(x + 24, y + 12, 250, 110),
            value,
            color,
            76,
            charging,
        );
    }
}
fn charging_phase(charging: bool, reduced: bool, seconds: f32) -> Option<f32> {
    (charging && !reduced).then(|| (seconds / 3.8).rem_euclid(1.))
}
unsafe fn battery_text(dc: HDC, r: RECT, value: &str, color: u32, size: i32, charging: bool) {
    unsafe {
        let s = state();
        let charging = charging && s.battery.is_some();
        let reduced = s.reduce_motion || view().os_reduce_motion;
        let seconds = CHARGE_EPOCH
            .get_or_init(Instant::now)
            .elapsed()
            .as_secs_f32();
        let phase = PREVIEW_CHARGE_PHASE
            .get()
            .copied()
            .filter(|_| charging && !reduced)
            .or_else(|| charging_phase(charging, reduced, seconds));
        let panel = crate::theme::current().panel;
        let color = crate::theme::readable_on(color, panel);
        if let Some(phase) = phase {
            let highlight = if crate::theme::light_surface(panel) {
                mix(color, 0, 0.25)
            } else {
                mix(color, 0xffffff, 0.6)
            };
            crate::render::charging_text(
                dc,
                r,
                value,
                faded(color),
                size,
                600,
                phase,
                faded(highlight),
            );
        } else {
            crate::render::text(dc, r, value, faded(color), size, 600, false);
        }
    }
}
fn rounded_alpha(x: f32, y: f32, width: f32, height: f32, radius: f32) -> u8 {
    let radius = radius.min(width / 2.).min(height / 2.);
    let dx = (x - width / 2.).abs() - (width / 2. - radius);
    let dy = (y - height / 2.).abs() - (height / 2. - radius);
    let distance = dx.max(0.).hypot(dy.max(0.)) + dx.max(dy).min(0.) - radius;
    ((0.5 - distance).clamp(0., 1.) * 255.).round() as u8
}
fn preview_alpha(pixels: &mut [u8], width: usize, height: usize) {
    let radius = 12. * (width as f32 / 290.).min(height as f32 / 100.);
    for y in 0..height {
        for x in 0..width {
            let alpha = rounded_alpha(
                x as f32 + 0.5,
                y as f32 + 0.5,
                width as f32,
                height as f32,
                radius,
            );
            let index = (y * width + x) * 4;
            for channel in 0..3 {
                pixels[index + channel] =
                    ((pixels[index + channel] as u16 * alpha as u16 + 127) / 255) as u8;
            }
            pixels[index + 3] = alpha;
        }
    }
}
unsafe fn paint_preview(hwnd: HWND) -> bool {
    unsafe {
        let mut r = zeroed();
        GetClientRect(hwnd, &mut r);
        let width = r.right.max(1);
        let height = r.bottom.max(1);
        let dc = CreateCompatibleDC(null_mut());
        let (bitmap, bits) = dib(dc, width, height);
        if bitmap.is_null() {
            DeleteDC(dc);
            return false;
        }
        let old = SelectObject(dc, bitmap);
        let target = crate::render::target(dc, width, height, 290, 100);
        paint(dc, &state(), true, -1, None, None, None);
        drop(target);
        GdiFlush();
        let pixels = std::slice::from_raw_parts_mut(bits, (width * height * 4) as usize);
        preview_alpha(pixels, width as usize, height as usize);
        let mut window_rect = zeroed();
        GetWindowRect(hwnd, &mut window_rect);
        let destination = POINT {
            x: window_rect.left,
            y: window_rect.top,
        };
        let source = POINT { x: 0, y: 0 };
        let size = SIZE {
            cx: width,
            cy: height,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let updated = UpdateLayeredWindow(
            hwnd,
            null_mut(),
            &destination,
            &size,
            dc,
            &source,
            0,
            &blend,
            ULW_ALPHA,
        ) != 0;
        PREVIEW_RESULT.store(if updated { 1 } else { 2 }, Ordering::Relaxed);
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
        updated
    }
}
fn percent(v: Option<u8>) -> String {
    v.map(|v| format!("{v}%")).unwrap_or_else(|| "—".into())
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum SliderKind {
    Game,
    Mic,
    Balance,
    Sidetone,
    Eq(usize),
}
struct Slider {
    r: RECT,
    kind: SliderKind,
    value: Option<i32>,
    min: i32,
    max: i32,
    vertical: bool,
}
enum Command {
    Page(u8),
    Action(Action),
    Preset(usize),
    ApplyEq,
    RevertEq,
    SleepEdit,
    ApplySleep,
    File,
    Help,
    Hide,
    Minimize,
    Refresh,
}
struct Button {
    r: RECT,
    label: String,
    command: Command,
    enabled: bool,
    selected: bool,
    primary: bool,
}
fn controls(s: &State, v: &View) -> Vec<Button> {
    let mut b = Vec::new();
    let mut add = |r, label: &str, command, enabled, selected, primary| {
        b.push(Button {
            r,
            label: label.into(),
            command,
            enabled,
            selected,
            primary,
        })
    };
    add(
        rect(96, 12, 48, 32),
        "File",
        Command::File,
        true,
        v.menu_open == Some(0),
        false,
    );
    add(
        rect(150, 12, 52, 32),
        "Help",
        Command::Help,
        true,
        v.menu_open == Some(2),
        false,
    );
    add(
        rect(918, 8, 44, 36),
        "\u{2212}",
        Command::Minimize,
        true,
        false,
        false,
    );
    add(rect(968, 8, 44, 36), "x", Command::Hide, true, false, false);
    for (i, name) in PAGE_NAMES.iter().enumerate() {
        add(
            rect(12, 150 + i as i32 * 54, sidebar_width(v) - 24, 44),
            name,
            Command::Page(i as u8),
            true,
            v.page as usize == i,
            false,
        );
    }
    add(
        rect(12, 650, sidebar_width(v) - 24, 44),
        "Refresh device",
        Command::Refresh,
        true,
        false,
        false,
    );
    match v.page {
        0 => {
            add(
                rect(448, 574, 136, 32),
                if s.audio.speaker_muted {
                    "Unmute output"
                } else {
                    "Mute output"
                },
                Command::Action(Action::SetSpeakerMute(!s.audio.speaker_muted)),
                s.audio.speaker.is_some(),
                s.audio.speaker_muted,
                false,
            );
            add(
                rect(840, 574, 144, 32),
                if s.audio.microphone_muted {
                    "Unmute mic"
                } else {
                    "Mute mic"
                },
                Command::Action(Action::SetMicMute(!s.audio.microphone_muted)),
                s.audio.microphone.is_some(),
                s.audio.microphone_muted,
                false,
            );
            add(
                rect(884, 325, 100, 30),
                "Reset mix",
                Command::Action(Action::SetGameChatBalance(10)),
                s.game_chat_balance.is_some(),
                false,
                false,
            );
        }
        1 => {
            let available = s.features.bands.is_some();
            for (i, (name, bands)) in crate::features::PRESETS.iter().enumerate() {
                add(
                    rect(236 + i as i32 * 111, 179, 101, 34),
                    name,
                    Command::Preset(i),
                    available,
                    v.draft.or(s.features.bands) == Some(*bands),
                    false,
                );
            }
            add(
                rect(738, 675, 110, 38),
                "Revert",
                Command::RevertEq,
                v.draft.is_some(),
                false,
                false,
            );
            add(
                rect(862, 675, 146, 38),
                "Apply equalizer",
                Command::ApplyEq,
                available
                    && v.draft.is_some_and(|bands| Some(bands) != s.features.bands)
                    && v.draft_slot == s.preset
                    && v.eq_pending.is_none(),
                false,
                true,
            );
        }
        2 => {
            add(
                rect(551, 235, 150, 34),
                if s.audio.microphone_muted {
                    "Unmute mic"
                } else {
                    "Mute mic"
                },
                Command::Action(Action::SetMicMute(!s.audio.microphone_muted)),
                s.audio.microphone.is_some(),
                s.audio.microphone_muted,
                false,
            );
            for (i, name) in ["Default", "Mic boost", "Broadcast", "Conference"]
                .iter()
                .enumerate()
            {
                add(
                    rect(256 + i as i32 * 112, 387, 104, 34),
                    name,
                    Command::Action(Action::SetMicPreset(32 + i as u8)),
                    s.mic_eq.is_some(),
                    s.mic_eq == Some(32 + i as u8),
                    false,
                );
            }
            add(
                rect(551, 604, 150, 34),
                if s.notifications_enabled {
                    "Alerts on"
                } else {
                    "Alerts off"
                },
                Command::Action(Action::SetMicNotifications(!s.notifications_enabled)),
                true,
                s.notifications_enabled,
                false,
            );
        }
        3 => {
            for (i, value) in [0, 15, 30, 60].iter().enumerate() {
                let name = if *value == 0 {
                    "Never".into()
                } else {
                    format!("{value} min")
                };
                add(
                    rect(256 + i as i32 * 83, 258, 75, 32),
                    &name,
                    Command::Action(Action::SetSleep(*value)),
                    s.features.sleep.is_some(),
                    s.features.sleep == Some(*value),
                    false,
                );
            }
            add(
                rect(256, 328, 154, 38),
                &v.minutes,
                Command::SleepEdit,
                s.features.sleep.is_some(),
                v.editing,
                false,
            );
            add(
                rect(424, 328, 160, 38),
                "Set sleep timer",
                Command::ApplySleep,
                s.features.sleep.is_some(),
                false,
                true,
            );
            for (i, (name, value)) in [("Maximum range", 0), ("Low latency", 1)]
                .iter()
                .enumerate()
            {
                add(
                    rect(648, 251 + i as i32 * 57, 340, 42),
                    name,
                    Command::Action(Action::SetWireless(*value)),
                    s.features.wireless.is_some(),
                    s.features.wireless == Some(*value),
                    false,
                );
            }
            for (i, (name, value)) in [("Off", 0), ("ANC", 1), ("Ambient", 80)].iter().enumerate() {
                add(
                    rect(256 + i as i32 * 111, 490, 103, 36),
                    name,
                    Command::Action(Action::SetAnc([
                        *value,
                        s.features.anc.map(|x| x[1]).unwrap_or(1),
                    ])),
                    s.features.anc.is_some(),
                    s.features.anc.is_some_and(|x| x[0] == *value),
                    false,
                );
            }
            add(
                rect(648, 490, 158, 36),
                "Disabled",
                Command::Action(Action::SetVoicePrompt(false)),
                s.features.voice_prompt.is_some(),
                s.features.voice_prompt == Some(false),
                false,
            );
            add(
                rect(820, 490, 168, 36),
                "Enabled",
                Command::Action(Action::SetVoicePrompt(true)),
                s.features.voice_prompt.is_some(),
                s.features.voice_prompt == Some(true),
                false,
            );
            for (i, name) in ["Connection", "Battery level", "Low battery"]
                .iter()
                .enumerate()
            {
                add(
                    rect(520 + i as i32 * 158, 647, 146, 34),
                    name,
                    Command::Action(Action::SetLed(i as u8)),
                    s.features.led.is_some(),
                    s.features.led == Some(i as u8),
                    false,
                );
            }
        }
        4 => {
            for (i, name) in crate::theme::NAMES.iter().enumerate() {
                add(
                    rect(879, 197 + i as i32 * 87, 108, 34),
                    if s.theme as usize == i {
                        "Selected"
                    } else {
                        "Use theme"
                    },
                    Command::Action(Action::SetTheme(i as u8)),
                    true,
                    s.theme as usize == i,
                    false,
                );
                let _ = name;
            }
            add(
                rect(827, 652, 160, 34),
                if v.os_reduce_motion {
                    "Reduced by Windows"
                } else if s.reduce_motion {
                    "Motion reduced"
                } else {
                    "Motion enabled"
                },
                Command::Action(Action::SetReduceMotion(!s.reduce_motion)),
                !v.os_reduce_motion,
                !s.reduce_motion && !v.os_reduce_motion,
                false,
            );
        }
        _ => {}
    }
    for button in &mut b {
        if button.r.top >= 100 && !matches!(button.command, Command::Page(_) | Command::Refresh) {
            button.r.left -= content_shift(v);
            button.r.right -= content_shift(v);
        }
    }
    b
}
fn sliders(s: &State, v: &View) -> Vec<Slider> {
    let mut out = Vec::new();
    let mut add = |r, kind, value, min, max, vertical| {
        out.push(Slider {
            r,
            kind,
            value,
            min,
            max,
            vertical,
        })
    };
    let game = v.overrides[0].or(s.audio.speaker.map(i32::from));
    let mic = v.overrides[1].or(s.audio.microphone.map(i32::from));
    let balance = v.overrides[2].or(s.game_chat_balance.map(i32::from));
    let side = v.overrides[3].or(s.sidetone.map(i32::from));
    match v.page {
        0 => {
            add(
                rect(256, 529, 328, 26),
                SliderKind::Game,
                game,
                0,
                100,
                false,
            );
            add(rect(648, 529, 336, 26), SliderKind::Mic, mic, 0, 100, false);
            add(
                rect(740, 271, 244, 28),
                SliderKind::Balance,
                balance,
                0,
                20,
                false,
            );
        }
        1 => {
            let bands = v.draft.or(s.features.bands);
            for i in 0..10 {
                add(
                    rect(280 + i as i32 * 70, 457, 24, 140),
                    SliderKind::Eq(i),
                    bands.map(|x| i32::from(x[i])),
                    -12,
                    12,
                    true,
                );
            }
        }
        2 => {
            add(rect(256, 293, 445, 28), SliderKind::Mic, mic, 0, 100, false);
            add(
                rect(256, 533, 445, 28),
                SliderKind::Sidetone,
                side,
                0,
                15,
                false,
            );
        }
        _ => {}
    }
    for slider in &mut out {
        slider.r.left -= content_shift(v);
        slider.r.right -= content_shift(v);
    }
    out
}
fn slider_value(slider: &Slider, x: i32, y: i32) -> i32 {
    let (distance, length) = if slider.vertical {
        (slider.r.bottom - y, slider.r.bottom - slider.r.top)
    } else {
        (x - slider.r.left, slider.r.right - slider.r.left)
    };
    let length = length.max(1);
    slider.min + (distance.clamp(0, length) * (slider.max - slider.min) + length / 2) / length
}
fn change_slider(kind: SliderKind, value: i32, s: &State) {
    let mut v = view();
    v.error = None;
    v.override_time = Instant::now();
    match kind {
        SliderKind::Game => {
            v.overrides[0] = Some(value);
            send(Action::SetSpeakerVolume(value as u8));
        }
        SliderKind::Mic => {
            v.overrides[1] = Some(value);
            send(Action::SetMicVolume(value as u8));
        }
        SliderKind::Balance => {
            v.overrides[2] = Some(value);
            send(Action::SetGameChatBalance(value as u8));
        }
        SliderKind::Sidetone => {
            v.overrides[3] = Some(value);
            send(Action::SetSidetone(value as u8));
        }
        SliderKind::Eq(index) => {
            if let Some(mut bands) = v.draft.or(s.features.bands) {
                bands[index] = value as i8;
                v.draft = Some(bands);
                v.draft_slot = s.preset;
            }
        }
    }
}
unsafe fn draw_slider(dc: HDC, slider: &Slider) {
    unsafe {
        let p = crate::theme::current();
        let r = slider.r;
        let rail = mix(p.panel, p.text, 0.16);
        if slider.vertical {
            let x = (r.left + r.right) / 2;
            round(dc, rect(x - 2, r.top, 4, r.bottom - r.top), rail, 4);
            let zero = r.bottom - (0 - slider.min) * (r.bottom - r.top) / (slider.max - slider.min);
            if let Some(value) = slider.value {
                let y = r.bottom
                    - (value - slider.min) * (r.bottom - r.top) / (slider.max - slider.min);
                round(
                    dc,
                    rect(x - 2, y.min(zero), 4, (y - zero).abs().max(2)),
                    p.accent,
                    4,
                );
                round(dc, rect(x - 10, y - 5, 20, 10), p.text, 6);
            }
        } else {
            let y = (r.top + r.bottom) / 2;
            round(dc, rect(r.left, y - 2, r.right - r.left, 4), rail, 4);
            if let Some(value) = slider.value {
                let x =
                    r.left + (value - slider.min) * (r.right - r.left) / (slider.max - slider.min);
                round(dc, rect(r.left, y - 2, x - r.left, 4), p.accent, 4);
                round(dc, rect(x - 6, y - 6, 12, 12), p.text, 12);
            }
        }
    }
}
unsafe fn draw_button(dc: HDC, b: &Button, focused: bool, hover: bool) {
    unsafe {
        let p = crate::theme::current();
        let sidebar = matches!(b.command, Command::Page(_));
        let menu = matches!(
            b.command,
            Command::File | Command::Help | Command::Hide | Command::Minimize
        );
        let color = if !b.enabled {
            mix(p.panel, p.text, 0.04)
        } else if b.primary {
            p.accent
        } else if b.selected {
            mix(p.panel, p.accent, 0.19)
        } else if hover && !matches!(b.command, Command::File | Command::Help) {
            mix(p.panel, p.text, 0.09)
        } else if sidebar || menu {
            p.bg
        } else {
            mix(p.panel, p.text, 0.05)
        };
        if focused && b.enabled {
            round(
                dc,
                rect(
                    b.r.left - 2,
                    b.r.top - 2,
                    b.r.right - b.r.left + 4,
                    b.r.bottom - b.r.top + 4,
                ),
                p.text,
                11,
            );
        }
        round(dc, b.r, color, 10);
        if sidebar && b.selected {
            round(dc, rect(b.r.left, b.r.top + 10, 3, 24), p.accent, 3);
        }
        if matches!(b.command, Command::SleepEdit) {
            let v = view();
            if v.editing {
                if v.select_all {
                    round(
                        dc,
                        rect(
                            b.r.left + 9,
                            b.r.top + 6,
                            (v.minutes.len() as i32 * 7 + 7).max(16),
                            b.r.bottom - b.r.top - 12,
                        ),
                        mix(p.panel, p.accent, 0.22),
                        4,
                    );
                } else if v.transition.elapsed().as_millis() % 1000 < 500 {
                    fill(
                        dc,
                        rect(
                            b.r.left + 12 + v.caret as i32 * 7,
                            b.r.top + 10,
                            1,
                            b.r.bottom - b.r.top - 20,
                        ),
                        p.text,
                    );
                }
            }
        }
        if let Command::Page(page) = b.command {
            let color = if b.selected { p.accent } else { p.muted };
            let x = b.r.left + 16;
            let y = b.r.top + 15;
            match page {
                0 => {
                    for (dx, dy) in [(0, 0), (8, 0), (0, 8), (8, 8)] {
                        round(dc, rect(x + dx, y + dy, 5, 5), color, 2);
                    }
                }
                1 => {
                    for (dx, dy) in [(0, 8), (6, 3), (12, 10)] {
                        line(
                            dc,
                            &[
                                POINT { x: x + dx, y },
                                POINT {
                                    x: x + dx,
                                    y: y + 15,
                                },
                            ],
                            color,
                            1.2,
                        );
                        round(dc, rect(x + dx - 2, y + dy, 5, 4), color, 2);
                    }
                }
                2 => {
                    round(dc, rect(x + 4, y, 7, 13), color, 7);
                    line(
                        dc,
                        &[
                            POINT { x, y: y + 7 },
                            POINT { x, y: y + 13 },
                            POINT {
                                x: x + 7,
                                y: y + 17,
                            },
                            POINT {
                                x: x + 14,
                                y: y + 13,
                            },
                            POINT {
                                x: x + 14,
                                y: y + 7,
                            },
                        ],
                        color,
                        1.2,
                    );
                }
                3 => {
                    round(dc, rect(x, y + 1, 15, 12), color, 4);
                    round(
                        dc,
                        rect(x + 2, y + 3, 11, 8),
                        if b.selected {
                            mix(p.panel, p.accent, 0.19)
                        } else {
                            p.bg
                        },
                        3,
                    );
                    round(dc, rect(x + 5, y + 16, 5, 2), color, 2);
                }
                _ => {
                    for (dx, dy) in [(1, 1), (10, 1), (1, 10), (10, 10)] {
                        round(dc, rect(x + dx, y + dy, 5, 5), color, 5);
                    }
                }
            }
        }
        let ink = if !b.enabled {
            mix(p.panel, p.text, 0.35)
        } else if b.primary {
            crate::theme::on_accent()
        } else if b.selected {
            p.accent
        } else {
            p.text
        };
        let navigation = sidebar || matches!(b.command, Command::Refresh);
        if matches!(b.command, Command::Refresh) {
            let x = b.r.left + 16;
            let y = b.r.top + 14;
            line(
                dc,
                &[
                    POINT { x, y: y + 5 },
                    POINT { x, y },
                    POINT { x: x + 15, y },
                    POINT {
                        x: x + 11,
                        y: y - 4,
                    },
                ],
                ink,
                1.5,
            );
            line(
                dc,
                &[
                    POINT {
                        x: x + 15,
                        y: y + 10,
                    },
                    POINT {
                        x: x + 15,
                        y: y + 15,
                    },
                    POINT { x, y: y + 15 },
                    POINT {
                        x: x + 4,
                        y: y + 19,
                    },
                ],
                ink,
                1.5,
            );
        }
        if navigation && b.r.right - b.r.left < 104 {
            return;
        }
        let ink = if navigation {
            mix(
                p.bg,
                ink,
                ((b.r.right - b.r.left - 104) as f32 / 76.0).clamp(0.0, 1.0),
            )
        } else {
            ink
        };
        text(
            dc,
            b.r.left + if navigation { 40 } else { 12 },
            b.r.top + 4,
            b.r.right - b.r.left - if navigation { 52 } else { 20 },
            b.r.bottom - b.r.top - 8,
            &b.label,
            ink,
            if sidebar { 14 } else { 12 },
            if b.selected || b.primary { 600 } else { 400 },
        );
    }
}

fn microphone_status(s: &State) -> &'static str {
    if s.hardware_mic_muted == Some(true) {
        "Muted on headset"
    } else if s.audio.microphone.is_none() {
        "Input unavailable"
    } else if s.audio.microphone_muted {
        "Software mute is on"
    } else if s.hardware_mic_muted.is_none() {
        "Headset status unavailable"
    } else {
        "Ready to speak"
    }
}
unsafe fn paint(
    dc: HDC,
    s: &State,
    compact: bool,
    _focus: i32,
    _mic: Option<u8>,
    _game: Option<u8>,
    _balance: Option<u8>,
) {
    unsafe {
        crate::theme::set(s.theme);
        FADE.store(1000, Ordering::Relaxed);
        let p = crate::theme::current();
        fill(
            dc,
            rect(
                0,
                0,
                if compact { 290 } else { W },
                if compact { 100 } else { H },
            ),
            p.bg,
        );
        if compact {
            fill(dc, rect(0, 0, 290, 100), p.panel);
            label(
                dc,
                16,
                9,
                252,
                if s.charging == Some(true) {
                    "SCYTHE / CHARGING"
                } else {
                    "SCYTHE / STATUS"
                },
            );
            battery_text(
                dc,
                rect(16, 30, 100, 44),
                &percent(s.battery),
                crate::battery::color(s.battery),
                32,
                s.charging == Some(true),
            );
            text(dc, 122, 33, 150, 24, microphone_status(s), p.text, 13, 600);
            text(
                dc,
                122,
                58,
                150,
                18,
                &format!("Game {}", percent(s.audio.speaker)),
                p.muted,
                11,
                400,
            );
            text(dc, 16, 79, 257, 16, &s.status, p.muted, 10, 400);
            return;
        }
        let v = view().clone();
        let buttons = controls(s, &v);
        fill(dc, rect(72, 52, W - 72, 1), mix(p.bg, p.text, 0.09));
        label(dc, 750, 15, 155, "BLACKSHARK V3 PRO");
        for (i, b) in buttons.iter().enumerate().filter(|(_, b)| b.r.top < 100) {
            draw_button(dc, b, v.keyboard_focus && v.focus == i, v.hover == Some(i));
        }
        let content_offset = crate::render::translate(dc, -content_shift(&v) as f32);
        let title = PAGE_NAMES[v.page.min(4) as usize];
        text(dc, 236, 85, 640, 43, title, p.text, 31, 600);
        let subtitle = match v.page {
            0 => "Everything you need. Nothing in your way.",
            1 => "Shape your sound with precise, hardware-backed equalization.",
            2 => "Your voice, level, and monitoring in one place.",
            3 => "Power, wireless, and the controls on your headset.",
            _ => "A workspace that feels like yours.",
        };
        text(dc, 236, 132, 772, 23, subtitle, p.muted, 13, 400);
        let elapsed = v.transition.elapsed().as_secs_f32() / 0.22;
        let progress = if s.reduce_motion || v.os_reduce_motion || _focus < 0 {
            1.0
        } else {
            1.0 - (1.0 - elapsed.clamp(0., 1.)).powi(3)
        };
        FADE.store((progress * 1000.) as u32, Ordering::Relaxed);
        match v.page {
            0 => {
                card(dc, rect(236, 180, 460, 242));
                label(
                    dc,
                    258,
                    197,
                    290,
                    if s.charging == Some(true) {
                        "BATTERY / CHARGING"
                    } else {
                        "BATTERY / WIRELESS HEADSET"
                    },
                );
                if progress > 0.98 {
                    percentage(
                        dc,
                        246,
                        217,
                        &percent(s.battery),
                        crate::battery::color(s.battery),
                        s.charging == Some(true),
                    );
                } else {
                    text(
                        dc,
                        270,
                        229,
                        300,
                        110,
                        &percent(s.battery),
                        crate::battery::color(s.battery),
                        76,
                        600,
                    );
                }
                let runtime = s
                    .estimated_minutes
                    .map(|m| format!("Approximately {}h {}m remaining", m / 60, m % 60))
                    .unwrap_or_else(|| {
                        if s.charging == Some(true) {
                            "Charging your headset".into()
                        } else {
                            "Learning your battery life".into()
                        }
                    });
                text(dc, 258, 350, 410, 24, &runtime, p.text, 15, 600);
                text(dc, 258, 382, 410, 20, &s.status, p.muted, 12, 400);
                card(dc, rect(716, 180, 292, 242));
                label(dc, 740, 197, 240, "GAME / CHAT BALANCE");
                text(
                    dc,
                    740,
                    230,
                    244,
                    26,
                    &s.game_chat_balance
                        .map(|x| {
                            if x == 10 {
                                "Balanced".into()
                            } else {
                                let (g, c) = crate::mixer::levels(x);
                                format!("Game {g}% / Chat {c}%")
                            }
                        })
                        .unwrap_or("Unavailable".into()),
                    p.text,
                    20,
                    600,
                );
                text(dc, 740, 303, 120, 20, "GAME", p.muted, 10, 600);
                text(dc, 914, 303, 70, 20, "CHAT", p.muted, 10, 600);
                text(
                    dc,
                    740,
                    372,
                    244,
                    20,
                    "Route voice apps to the Chat output.",
                    p.muted,
                    11,
                    400,
                );
                for (x, w, title, detail, value) in [
                    (
                        236,
                        372,
                        "Game output",
                        "Headphone volume",
                        percent(s.audio.speaker),
                    ),
                    (
                        628,
                        380,
                        "Microphone",
                        microphone_status(s),
                        percent(s.audio.microphone),
                    ),
                ] {
                    card(dc, rect(x, 442, w, 230));
                    label(dc, x + 20, 459, w - 40, title);
                    text(dc, x + 20, 489, 180, 36, &value, p.text, 28, 600);
                    text(dc, x + 20, 576, 170, 24, detail, p.muted, 12, 400);
                }
                text(
                    dc,
                    256,
                    632,
                    328,
                    19,
                    "Volume changes apply immediately.",
                    p.muted,
                    11,
                    400,
                );
                text(
                    dc,
                    648,
                    632,
                    336,
                    19,
                    if s.hardware_mic_muted == Some(true) {
                        "Release physical mute on your headset."
                    } else {
                        "Physical mute and software mute stay separate."
                    },
                    p.muted,
                    11,
                    400,
                );
            }
            1 => {
                let bands = v.draft.or(s.features.bands);
                card(dc, rect(236, 232, 772, 425));
                label(dc, 256, 246, 280, "EQUALIZER CURVE");
                text(
                    dc,
                    760,
                    246,
                    228,
                    20,
                    &s.preset
                        .map(|slot| format!("HEADSET SLOT {slot}"))
                        .unwrap_or("HEADSET OFFLINE".into()),
                    p.muted,
                    11,
                    600,
                );
                let left = 292;
                let right = 922;
                let top = 288;
                let bottom = 420;
                for (db, y) in [
                    (12, top),
                    (6, top + 33),
                    (0, top + 66),
                    (-6, top + 99),
                    (-12, bottom),
                ] {
                    line(
                        dc,
                        &[POINT { x: left, y }, POINT { x: right, y }],
                        mix(p.panel, p.text, if db == 0 { 0.18 } else { 0.07 }),
                        1.,
                    );
                    text(dc, 251, y - 9, 35, 18, &format!("{db:+}"), p.muted, 10, 400);
                }
                for i in 0..10 {
                    let x = 292 + i * 70;
                    line(
                        dc,
                        &[POINT { x, y: top }, POINT { x, y: bottom }],
                        mix(p.panel, p.text, 0.055),
                        1.,
                    );
                }
                if let Some(bands) = bands {
                    let points: Vec<POINT> = bands
                        .iter()
                        .enumerate()
                        .map(|(i, db)| POINT {
                            x: 292 + i as i32 * 70,
                            y: 354 - *db as i32 * 132 / 24,
                        })
                        .collect();
                    let mut area = vec![POINT { x: left, y: bottom }];
                    area.extend_from_slice(&points);
                    area.push(POINT {
                        x: right,
                        y: bottom,
                    });
                    crate::render::polygon(dc, &area, faded(mix(p.panel, p.accent, 0.06)));
                    line(dc, &points, p.accent, 2.2);
                    for point in &points {
                        round(dc, rect(point.x - 3, point.y - 3, 6, 6), p.accent, 6);
                    }
                }
                for (i, freq) in crate::features::FREQUENCIES.iter().enumerate() {
                    let x = 268 + i as i32 * 70;
                    text(
                        dc,
                        x,
                        433,
                        48,
                        22,
                        &bands
                            .map(|b| format!("{:+} dB", b[i]))
                            .unwrap_or("--".into()),
                        p.text,
                        12,
                        600,
                    );
                    text(dc, x, 608, 50, 21, freq, p.muted, 11, 400);
                }
                label(dc, 256, 632, 650, "10 BANDS / 31 HZ - 16 KHZ / +/-12 DB");
                let status = if v.eq_pending.is_some() {
                    "Applying equalizer..."
                } else if s.features.bands.is_none() {
                    "Waiting for headset"
                } else if v.draft.is_some_and(|bands| Some(bands) != s.features.bands) {
                    if v.draft_slot == s.preset {
                        "Unsaved changes"
                    } else {
                        "Headset slot changed. Revert and edit again."
                    }
                } else {
                    "Device values loaded"
                };
                text(dc, 236, 674, 490, 22, status, p.text, 13, 600);
                text(
                    dc,
                    236,
                    699,
                    490,
                    18,
                    "Apply replaces this slot's bands. Other slots are preserved.",
                    p.muted,
                    11,
                    400,
                );
            }
            2 => {
                card(dc, rect(236, 180, 490, 290));
                label(dc, 256, 197, 445, "MICROPHONE INPUT");
                text(
                    dc,
                    256,
                    232,
                    260,
                    43,
                    &percent(s.audio.microphone),
                    p.text,
                    32,
                    600,
                );
                text(
                    dc,
                    256,
                    332,
                    445,
                    23,
                    microphone_status(s),
                    p.muted,
                    13,
                    400,
                );
                label(dc, 256, 361, 440, "MICROPHONE PRESET");
                text(
                    dc,
                    256,
                    435,
                    445,
                    20,
                    "Hardware EQ presets apply immediately.",
                    p.muted,
                    11,
                    400,
                );
                card(dc, rect(236, 490, 490, 182));
                label(dc, 256, 507, 445, "SIDETONE / HEAR YOUR OWN VOICE");
                text(
                    dc,
                    256,
                    570,
                    260,
                    23,
                    &s.sidetone
                        .map(|x| format!("Monitoring level {x} / 15"))
                        .unwrap_or("Unavailable".into()),
                    p.text,
                    14,
                    600,
                );
                text(
                    dc,
                    256,
                    609,
                    280,
                    26,
                    "Headset mute notifications",
                    p.muted,
                    13,
                    400,
                );
                card(dc, rect(746, 180, 262, 492));
                label(dc, 766, 197, 222, "VOICE PROCESSING");
                text(dc, 766, 235, 222, 29, "Enhancements", p.text, 20, 600);
                for (i, title) in ["Normalization", "Vocal clarity", "Noise reduction"]
                    .iter()
                    .enumerate()
                {
                    text(
                        dc,
                        766,
                        300 + i as i32 * 63,
                        222,
                        25,
                        title,
                        p.muted,
                        14,
                        400,
                    );
                    line(
                        dc,
                        &[
                            POINT {
                                x: 766,
                                y: 342 + i as i32 * 63,
                            },
                            POINT {
                                x: 988,
                                y: 342 + i as i32 * 63,
                            },
                        ],
                        mix(p.panel, p.text, 0.08),
                        1.,
                    );
                }
                text(
                    dc,
                    766,
                    550,
                    222,
                    25,
                    "Not available in this version.",
                    p.muted,
                    12,
                    400,
                );
                text(dc, 766, 582, 222, 25, "", p.muted, 11, 400);
            }
            3 => {
                for (x, w, title, detail) in [
                    (236, 372, "Auto sleep", "Power off after silence."),
                    (
                        628,
                        380,
                        "Wireless mode",
                        "Prioritize distance or response time.",
                    ),
                ] {
                    card(dc, rect(x, 180, w, 215));
                    label(dc, x + 20, 197, w - 40, title);
                    text(dc, x + 20, 225, w - 40, 22, detail, p.muted, 12, 400);
                }
                label(dc, 256, 304, 330, "CUSTOM MINUTES / 1-255");
                text(
                    dc,
                    256,
                    371,
                    328,
                    17,
                    &s.features
                        .sleep
                        .map(|x| {
                            if x == 0 {
                                "Timer disabled".into()
                            } else {
                                format!("Current timer: {x} minutes")
                            }
                        })
                        .unwrap_or("Connect to read timer".into()),
                    p.muted,
                    11,
                    400,
                );
                text(
                    dc,
                    648,
                    368,
                    340,
                    18,
                    "Changes may briefly reconnect the dongle.",
                    p.muted,
                    11,
                    400,
                );
                card(dc, rect(236, 415, 372, 180));
                label(dc, 256, 432, 332, "NOISE CANCELLATION");
                text(
                    dc,
                    256,
                    460,
                    332,
                    22,
                    "Choose your listening environment.",
                    p.muted,
                    12,
                    400,
                );
                text(
                    dc,
                    256,
                    553,
                    332,
                    20,
                    "Your current ANC strength is preserved.",
                    p.muted,
                    11,
                    400,
                );
                card(dc, rect(628, 415, 380, 180));
                label(dc, 648, 432, 340, "MUTE VOICE PROMPT");
                text(
                    dc,
                    648,
                    460,
                    340,
                    22,
                    "Spoken feedback from your headset.",
                    p.muted,
                    12,
                    400,
                );
                text(
                    dc,
                    648,
                    553,
                    340,
                    20,
                    "Applies to microphone mute / unmute.",
                    p.muted,
                    11,
                    400,
                );
                card(dc, rect(236, 615, 772, 90));
                label(dc, 256, 631, 245, "DONGLE INDICATOR");
                text(
                    dc,
                    256,
                    659,
                    245,
                    20,
                    "RGB effects are unsupported.",
                    p.muted,
                    11,
                    400,
                );
            }
            4 => {
                for (i, name) in crate::theme::NAMES.iter().enumerate() {
                    let y = 180 + i as i32 * 87;
                    card(dc, rect(236, y, 772, 73));
                    text(dc, 256, y + 17, 220, 30, name, p.text, 18, 600);
                    for (j, color) in crate::theme::SWATCHES[i].iter().enumerate() {
                        round(
                            dc,
                            rect(564 + j as i32 * 56, y + 21, 42, 30),
                            crate::theme::swatch(*color),
                            8,
                        );
                    }
                }
                card(dc, rect(236, 632, 772, 75));
                label(dc, 256, 645, 500, "INTERFACE MOTION");
                text(
                    dc,
                    256,
                    672,
                    500,
                    20,
                    "Subtle page fades. Reduce motion whenever you prefer.",
                    p.muted,
                    12,
                    400,
                );
            }
            _ => {}
        }
        drop(content_offset);
        for (index, slider) in sliders(s, &v).iter().enumerate() {
            if v.keyboard_focus && v.focus == buttons.len() + index && slider.value.is_some() {
                round(
                    dc,
                    rect(
                        slider.r.left - 7,
                        slider.r.top - 6,
                        slider.r.right - slider.r.left + 14,
                        slider.r.bottom - slider.r.top + 12,
                    ),
                    p.text,
                    10,
                );
                round(
                    dc,
                    rect(
                        slider.r.left - 6,
                        slider.r.top - 5,
                        slider.r.right - slider.r.left + 12,
                        slider.r.bottom - slider.r.top + 10,
                    ),
                    p.panel,
                    9,
                );
            }
            draw_slider(dc, slider);
        }
        for (i, b) in buttons.iter().enumerate().filter(|(_, b)| {
            b.r.top >= 100 && !matches!(b.command, Command::Page(_) | Command::Refresh)
        }) {
            draw_button(dc, b, v.keyboard_focus && v.focus == i, v.hover == Some(i));
        }
        paint_chrome(dc, s, &v, &buttons);
    }
}
unsafe fn paint_chrome(dc: HDC, s: &State, v: &View, buttons: &[Button]) {
    unsafe {
        let p = crate::theme::current();
        FADE.store(1000, Ordering::Relaxed);
        fill(dc, rect(72, 0, W - 72, 53), p.bg);
        fill(dc, rect(72, 52, W - 72, 1), mix(p.bg, p.text, 0.09));
        fill(dc, rect(72, 732, W - 72, H - 732), p.bg);
        label(dc, 750, 15, 155, "BLACKSHARK V3 PRO");
        for (i, b) in buttons.iter().enumerate().filter(|(_, b)| b.r.top < 100) {
            draw_button(dc, b, v.keyboard_focus && v.focus == i, v.hover == Some(i));
        }
        FADE.store(1000, Ordering::Relaxed);
        fill(dc, rect(72, 731, W - 72, 1), mix(p.bg, p.text, 0.08));
        let error = v.error.as_deref().or(s.last_error.as_deref());
        text(
            dc,
            236 - content_shift(v),
            734,
            772,
            23,
            error.unwrap_or("F1-F5  Navigate     Tab  Focus     Esc  Hide to tray"),
            if error.is_some() { p.accent } else { p.muted },
            10,
            400,
        );
        let width = sidebar_width(v);
        // Keep the expanding panel opaque at every animation frame.
        fill(dc, rect(0, 52, width, H - 52), p.bg);
        fill(dc, rect(0, 0, 72, 52), p.bg);
        fill(dc, rect(width - 1, 52, 1, H - 52), mix(p.bg, p.text, 0.14));
        round(
            dc,
            rect(29, 590, 12, 12),
            if s.battery.is_some() {
                crate::battery::color(Some(100))
            } else {
                p.surface
            },
            12,
        );
        text(dc, 25, 24, 28, 32, "S", p.accent, 23, 600);
        if v.sidebar > 0.25 {
            let opacity = ((v.sidebar - 0.25) / 0.75).clamp(0.0, 1.0);
            text(
                dc,
                28,
                70,
                width - 56,
                32,
                "SCYTHE",
                mix(p.bg, p.text, opacity),
                22,
                600,
            );
            text(
                dc,
                28,
                111,
                width - 56,
                20,
                "WORKSPACE",
                mix(p.bg, p.muted, opacity),
                10,
                600,
            );
            text(
                dc,
                28,
                714,
                width - 56,
                20,
                concat!("VERSION ", env!("CARGO_PKG_VERSION")),
                mix(p.bg, p.muted, opacity),
                10,
                400,
            );
        }
        for (i, b) in buttons
            .iter()
            .enumerate()
            .filter(|(_, b)| matches!(b.command, Command::Page(_) | Command::Refresh))
        {
            draw_button(dc, b, v.keyboard_focus && v.focus == i, v.hover == Some(i));
        }
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
unsafe fn update_battery_icon(hwnd: HWND, battery: Option<u8>) {
    unsafe {
        let color = crate::battery::color(battery);
        if color == ui(hwnd).icon_color {
            return;
        }
        let next = make_icon(color);
        if next.is_null() {
            return;
        }
        let previous = ui(hwnd).icon;
        ui(hwnd).icon = next;
        ui(hwnd).icon_color = color;
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, next as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, next as isize);
        notify(hwnd, NIM_MODIFY, false);
        if !previous.is_null() {
            DestroyIcon(previous);
        }
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
        if !activate {
            paint_preview(hwnd);
        }
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
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
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

unsafe fn animate(hwnd: HWND) {
    unsafe {
        view().transition = Instant::now();
        SetTimer(hwnd, 3, 16, None);
        InvalidateRect(hwnd, null(), 0);
    }
}
unsafe fn navigate(hwnd: HWND, page: u8) {
    unsafe {
        let mut v = view();
        v.page = page.min(4);
        if page == 3 && !v.minutes_dirty {
            v.minutes = state()
                .features
                .sleep
                .filter(|x| *x > 0)
                .unwrap_or(15)
                .to_string();
            v.caret = v.minutes.len();
        }
        v.focus = usize::MAX;
        v.hover = None;
        v.editing = false;
        v.error = None;
        drop(v);
        animate(hwnd);
        accessibility::page_changed(hwnd);
    }
}
unsafe fn paste_minutes(hwnd: HWND) {
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return;
        }
        let handle = GetClipboardData(13);
        if !handle.is_null() {
            let pointer = GlobalLock(handle);
            if !pointer.is_null() {
                let length = (GlobalSize(handle) / 2).min(4096);
                let units = std::slice::from_raw_parts(pointer.cast::<u16>(), length);
                let end = units.iter().position(|c| *c == 0).unwrap_or(length);
                let text = String::from_utf16_lossy(&units[..end]);
                let text = text.trim();
                let mut v = view();
                if !text.is_empty() && text.len() <= 6 && text.bytes().all(|b| b.is_ascii_digit()) {
                    v.minutes = text.into();
                    v.minutes_dirty = true;
                    v.caret = v.minutes.len();
                    v.select_all = false;
                    v.error = None;
                } else {
                    v.error = Some("Paste whole minutes from 1 to 255.".into());
                }
                GlobalUnlock(handle);
            }
        }
        CloseClipboard();
    }
}
unsafe fn popup(hwnd: HWND, kind: u8) {
    unsafe {
        let menu = CreatePopupMenu();
        if kind == 0 {
            AppendMenuW(menu, MF_STRING, 1, wide("Hide to tray\tEsc").as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
            AppendMenuW(menu, MF_STRING, 3, wide("Quit Scythe\tAlt+F4").as_ptr());
        } else {
            AppendMenuW(menu, MF_STRING, 2, wide("About Scythe").as_ptr());
            AppendMenuW(menu, MF_STRING, 21, wide("Keyboard shortcuts").as_ptr());
        }
        let mut client: RECT = zeroed();
        GetClientRect(hwnd, &mut client);
        let mut point = POINT {
            x: (if kind == 0 { 96 } else { 150 }) * client.right / W,
            y: 43 * client.bottom / H,
        };
        ClientToScreen(hwnd, &mut point);
        view().menu_open = Some(kind);
        InvalidateRect(hwnd, null(), 0);
        UpdateWindow(hwnd);
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY,
            point.x,
            point.y,
            0,
            hwnd,
            null(),
        );
        DestroyMenu(menu);
        {
            let mut v = view();
            v.hover = None;
            v.menu_open = None;
        }
        InvalidateRect(hwnd, null(), 0);
        UpdateWindow(hwnd);
        match selected {
            1 => {
                ShowWindow(hwnd, SW_HIDE);
            }
            2 => {
                about::show(hwnd);
            }
            3 => {
                DestroyWindow(hwnd);
            }
            10..=14 => navigate(hwnd, (selected - 10) as u8),
            20 => send(Action::SetReduceMotion(!state().reduce_motion)),
            21 => {
                MessageBoxW(hwnd,wide("F1-F5: Navigate sections\nF6: Switch navigation / page controls\nUp / Down: Move between navigation items\nTab / Shift+Tab: Move focus\nArrow keys: Adjust a focused slider\nHome / End: Slider endpoints\nEnter / Space: Activate a control\nEsc: Hide to tray\nAlt+F / Alt+H: Menus\nAlt+F4: Quit\n\nCustom sleep: whole minutes, 1-255.\nEqualizer edits apply only when you choose Apply.").as_ptr(),wide("Scythe shortcuts").as_ptr(),MB_OK);
            }
            _ => {}
        }
    }
}
unsafe fn activate(hwnd: HWND, command: Command) {
    unsafe {
        let s = state();
        view().error = None;
        match command {
            Command::Page(page) => {
                let keyboard = view().nav_keyboard;
                navigate(hwnd, page);
                if keyboard {
                    let mut v = view();
                    v.focus = 4 + page as usize;
                    v.nav_keyboard = true;
                }
            }
            Command::Action(action) => {
                let is_theme = matches!(action, Action::SetTheme(_));
                send(action);
                if is_theme {
                    animate(hwnd);
                }
            }
            Command::Preset(index) => {
                if s.features.bands.is_some() {
                    let mut v = view();
                    v.draft = Some(crate::features::PRESETS[index].1);
                    v.draft_slot = s.preset;
                }
            }
            Command::ApplyEq => {
                let mut v = view();
                if let Some(bands) = v.draft
                    && v.draft_slot == s.preset
                    && let Some(slot) = v.draft_slot
                {
                    v.eq_pending = Some((Instant::now(), slot, bands));
                    send(Action::SetSpeakerBands { slot, bands });
                }
            }
            Command::RevertEq => {
                let mut v = view();
                v.draft = None;
                v.draft_slot = None;
            }
            Command::SleepEdit => {
                let mut v = view();
                v.editing = true;
                v.select_all = true;
                v.caret = v.minutes.len();
            }
            Command::ApplySleep => {
                if s.features.sleep.is_none() {
                    view().error =
                        Some("Connect the headset before setting its sleep timer.".into());
                    return;
                }
                let minutes = view().minutes.clone();
                match crate::features::parse_minutes(&minutes) {
                    Ok(value) => {
                        send(Action::SetSleep(value));
                        view().editing = false;
                    }
                    Err(error) => view().error = Some(error.into()),
                }
            }
            Command::File => popup(hwnd, 0),
            Command::Help => popup(hwnd, 2),
            Command::Hide => {
                ShowWindow(hwnd, SW_HIDE);
            }
            Command::Minimize => {
                ShowWindow(hwnd, SW_MINIMIZE);
            }
            Command::Refresh => REFRESH.get().unwrap().store(true, Ordering::Relaxed),
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
            ANIMATION_FRAME => {
                FRAME_PENDING.store(false, Ordering::Release);
                if !ui(hwnd).animation_timer.is_null() {
                    SendMessageW(hwnd, WM_TIMER, 5, 0);
                }
                0
            }
            msg if msg == WM_APP + 20 => {
                let s = state();
                let v = view().clone();
                if v.page as isize == l
                    && let Some(button) = controls(&s, &v).into_iter().nth(w)
                    && button.enabled
                {
                    activate(hwnd, button.command);
                }
                0
            }
            WM_GETOBJECT if l as i32 == OBJID_CLIENT => accessibility::object(hwnd, w),
            WM_SETFOCUS => {
                accessibility::focus_changed(hwnd);
                0
            }
            WM_SETTINGCHANGE => {
                refresh_motion_preference();
                SetTimer(hwnd, 5, 16, None);
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_KILLFOCUS => {
                {
                    let mut v = view();
                    v.nav_keyboard = false;
                    v.hover = None;
                    v.keyboard_focus = false;
                }
                SetTimer(hwnd, 5, 16, None);
                InvalidateRect(hwnd, null(), 0);
                0
            }
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
                    WM_LBUTTONUP => {
                        show(hwnd);
                        animate(hwnd);
                    }
                    WM_RBUTTONUP => {
                        ShowWindow(ui(hwnd).tip, SW_HIDE);
                        let menu = CreatePopupMenu();
                        AppendMenuW(menu, MF_STRING, 1, wide("Open Scythe").as_ptr());
                        AppendMenuW(menu, MF_STRING, 2, wide("Refresh device").as_ptr());
                        AppendMenuW(menu, MF_SEPARATOR, 0, null());
                        AppendMenuW(menu, MF_STRING, 3, wide("Quit Scythe").as_ptr());
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
                            1 => {
                                show(hwnd);
                                animate(hwnd);
                            }
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
                if w == 5 {
                    let s = state();
                    let mut cursor = POINT::default();
                    GetCursorPos(&mut cursor);
                    ScreenToClient(hwnd, &mut cursor);
                    let mut bounds = RECT::default();
                    GetClientRect(hwnd, &mut bounds);
                    let x = cursor.x * W / bounds.right.max(1);
                    let y = cursor.y * H / bounds.bottom.max(1);
                    let mut v = view();
                    let inside = IsWindowVisible(hwnd) != 0
                        && IsIconic(hwnd) == 0
                        && x >= 0
                        && x < sidebar_width(&v)
                        && (0..H).contains(&y);
                    let open = inside || v.nav_keyboard;
                    let previous = v.sidebar;
                    update_sidebar(&mut v, open, s.reduce_motion);
                    let changed = previous != v.sidebar;
                    let settled = v.sidebar == if open { 1.0 } else { 0.0 };
                    animation_clock(hwnd, !settled);
                    if settled && (!open || v.nav_keyboard) {
                        KillTimer(hwnd, 5);
                    } else if settled || ui(hwnd).animation_timer.is_null() {
                        SetTimer(hwnd, 5, if settled { 80 } else { 10 }, None);
                    } else {
                        KillTimer(hwnd, 5);
                    }
                    if !inside && !v.nav_keyboard {
                        v.hover = None;
                    }
                    drop(v);
                    if changed {
                        InvalidateRect(hwnd, null(), 0);
                        UpdateWindow(hwnd);
                    }
                } else if w == 1 {
                    let mut p = zeroed();
                    GetCursorPos(&mut p);
                    if !tray_rect(hwnd).is_some_and(|r| PtInRect(&r, p) != 0) {
                        ShowWindow(ui(hwnd).tip, SW_HIDE);
                    }
                } else if w == 4 {
                    if IsWindowVisible(hwnd) != 0 && view().page == 0 {
                        InvalidateRect(hwnd, null(), 0);
                    }
                    if IsWindowVisible(ui(hwnd).tip) != 0 {
                        InvalidateRect(ui(hwnd).tip, null(), 0);
                    }
                } else if w == 3 {
                    if view().transition.elapsed().as_millis() > 240 || state().reduce_motion {
                        KillTimer(hwnd, 3);
                    }
                    if IsWindowVisible(hwnd) != 0 {
                        InvalidateRect(hwnd, null(), 0);
                    }
                } else {
                    let s = state();
                    let v = view().clone();
                    let charging_animation = s.charging == Some(true)
                        && s.battery.is_some()
                        && !s.reduce_motion
                        && !v.os_reduce_motion
                        && ((IsWindowVisible(hwnd) != 0 && v.page == 0)
                            || IsWindowVisible(ui(hwnd).tip) != 0);
                    if charging_animation != ui(hwnd).charging_animation {
                        ui(hwnd).charging_animation = charging_animation;
                        if charging_animation {
                            SetTimer(hwnd, 4, 33, None);
                        } else {
                            KillTimer(hwnd, 4);
                        }
                    }
                    update_battery_icon(hwnd, s.battery);
                    accessibility::poll(hwnd);
                    {
                        let actual = [
                            s.audio.speaker.map(i32::from),
                            s.audio.microphone.map(i32::from),
                            s.game_chat_balance.map(i32::from),
                            s.sidetone.map(i32::from),
                        ];
                        let mut v = view();
                        let expired = v.override_time.elapsed().as_millis() > 1000;
                        if let Some((started, slot, bands)) = v.eq_pending {
                            if s.preset == Some(slot) && s.features.bands == Some(bands) {
                                v.eq_pending = None;
                                if v.draft == Some(bands) && v.draft_slot == Some(slot) {
                                    v.draft = None;
                                    v.draft_slot = None;
                                }
                            } else if started.elapsed().as_secs() > 8
                                || (started.elapsed().as_millis() > 250 && s.last_error.is_some())
                            {
                                v.eq_pending = None;
                            }
                        }

                        for (index, value) in actual.iter().enumerate() {
                            if expired || v.overrides[index] == *value {
                                v.overrides[index] = None;
                            }
                        }
                    }
                    if ui(hwnd)
                        .baseline
                        .is_some_and(|old| old != s.mic_mute_notifications)
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
                if y < 52 && (x < 72 || (210..910).contains(&x)) {
                    HTCAPTION as isize
                } else {
                    HTCLIENT as isize
                }
            }
            WM_LBUTTONDOWN => {
                view().keyboard_focus = false;
                view().focus = usize::MAX;
                SetFocus(hwnd);
                let (x, y) = point(hwnd, l);
                let s = state();
                let v = view().clone();
                let buttons = controls(&s, &v);
                for (index, slider) in sliders(&s, &v).iter().enumerate() {
                    let hit = rect(
                        slider.r.left - 8,
                        slider.r.top - 5,
                        slider.r.right - slider.r.left + 16,
                        slider.r.bottom - slider.r.top + 10,
                    );
                    if contains(hit, x, y) && slider.value.is_some() {
                        ui(hwnd).drag = Some(slider.kind);
                        view().focus = buttons.len() + index;
                        view().editing = false;
                        SetCapture(hwnd);
                        change_slider(slider.kind, slider_value(slider, x, y), &s);
                        InvalidateRect(hwnd, null(), 0);
                        return 0;
                    }
                }
                view().editing = false;
                for (index, button) in buttons.into_iter().enumerate() {
                    if contains(button.r, x, y) {
                        view().focus = index;
                        if button.enabled {
                            activate(hwnd, button.command);
                        }
                        break;
                    }
                }
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_SETCURSOR if (l as u32 & 0xffff) == HTCLIENT => {
                let mut cursor = zeroed();
                GetCursorPos(&mut cursor);
                ScreenToClient(hwnd, &mut cursor);
                let mut bounds = zeroed();
                GetClientRect(hwnd, &mut bounds);
                let x = cursor.x * W / bounds.right.max(1);
                let y = cursor.y * H / bounds.bottom.max(1);
                let s = state();
                let v = view().clone();
                let mut shape = IDC_ARROW;
                for button in controls(&s, &v) {
                    if button.enabled && contains(button.r, x, y) {
                        shape = if matches!(button.command, Command::SleepEdit) {
                            IDC_IBEAM
                        } else {
                            IDC_HAND
                        };
                    }
                }
                for slider in sliders(&s, &v) {
                    if slider.value.is_some()
                        && contains(
                            rect(
                                slider.r.left - 8,
                                slider.r.top - 5,
                                slider.r.right - slider.r.left + 16,
                                slider.r.bottom - slider.r.top + 10,
                            ),
                            x,
                            y,
                        )
                    {
                        shape = if slider.vertical {
                            IDC_SIZENS
                        } else {
                            IDC_SIZEWE
                        };
                    }
                }
                SetCursor(LoadCursorW(null_mut(), shape));
                1
            }
            windows_sys::Win32::UI::Controls::WM_MOUSELEAVE => {
                view().hover = None;
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_MOUSEMOVE => {
                use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut tracking);
                let (x, y) = point(hwnd, l);
                let s = state();
                {
                    let mut v = view();
                    v.nav_keyboard = false;
                    let open = x >= 0 && x < sidebar_width(&v) && (0..H).contains(&y);
                    update_sidebar(&mut v, open, s.reduce_motion);
                    if open || v.sidebar > 0.0 {
                        SetTimer(hwnd, 5, 16, None);
                    }
                }
                let v = view().clone();
                if let Some(kind) = ui(hwnd).drag {
                    if let Some(slider) = sliders(&s, &v).iter().find(|sl| sl.kind == kind) {
                        change_slider(kind, slider_value(slider, x, y), &s);
                        InvalidateRect(hwnd, null(), 0);
                    }
                } else {
                    let hover = controls(&s, &v)
                        .iter()
                        .position(|b| contains(b.r, x, y) && b.enabled);
                    if hover != v.hover {
                        view().hover = hover;
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
                0
            }
            WM_LBUTTONUP | WM_CAPTURECHANGED => {
                if ui(hwnd).drag.take().is_some() {
                    ReleaseCapture();
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_SYSCHAR => 0,
            WM_SYSKEYDOWN => {
                view().keyboard_focus = true;
                match w {
                    0x46 => popup(hwnd, 0),
                    0x48 => popup(hwnd, 2),
                    _ => return DefWindowProcW(hwnd, msg, w, l),
                }
                0
            }
            WM_KEYDOWN => {
                view().keyboard_focus = true;
                let s = state();
                let v = view().clone();
                let buttons = controls(&s, &v);
                let sliders = sliders(&s, &v);
                if w == 0x1b {
                    ShowWindow(hwnd, SW_HIDE);
                } else if (0x70..=0x74).contains(&w) {
                    navigate(hwnd, (w - 0x70) as u8);
                } else if w == 0x75 {
                    let mut current = view();
                    let navigation = !v.nav_keyboard;
                    current.focus = if navigation {
                        4 + v.page as usize
                    } else {
                        buttons
                            .iter()
                            .enumerate()
                            .skip(10)
                            .find(|(_, b)| b.enabled)
                            .map(|(i, _)| i)
                            .or_else(|| {
                                sliders
                                    .iter()
                                    .position(|s| s.value.is_some())
                                    .map(|i| buttons.len() + i)
                            })
                            .unwrap_or(0)
                    };
                    current.nav_keyboard = navigation;
                    current.editing = false;
                    update_sidebar(&mut current, navigation, s.reduce_motion);
                    SetTimer(hwnd, 5, 16, None);
                } else if w == 9 {
                    let total = buttons.len() + sliders.len();
                    let reverse = GetKeyState(0x10) < 0;
                    let current = if v.focus < total {
                        v.focus
                    } else if reverse {
                        0
                    } else {
                        total - 1
                    };
                    let mut next = current;
                    for _ in 0..total {
                        next = if reverse {
                            (next + total - 1) % total
                        } else {
                            (next + 1) % total
                        };
                        if next >= buttons.len() {
                            if sliders[next - buttons.len()].value.is_some() {
                                break;
                            }
                        } else if buttons[next].enabled {
                            break;
                        }
                    }
                    let edit =
                        next < buttons.len() && matches!(buttons[next].command, Command::SleepEdit);
                    let mut current = view();
                    current.focus = next;
                    current.editing = edit;
                    current.nav_keyboard = next < buttons.len()
                        && matches!(buttons[next].command, Command::Page(_) | Command::Refresh);
                    let open = current.nav_keyboard;
                    update_sidebar(&mut current, open, s.reduce_motion);
                    SetTimer(hwnd, 5, 16, None);

                    if edit {
                        current.select_all = true;
                        current.caret = current.minutes.len();
                    }
                } else if v.focus < buttons.len()
                    && matches!(buttons[v.focus].command, Command::Page(_))
                    && matches!(w, 0x26 | 0x28 | 0x24 | 0x23)
                {
                    let first = buttons
                        .iter()
                        .position(|b| matches!(b.command, Command::Page(_)))
                        .unwrap();
                    let current = v.focus - first;
                    view().focus = first
                        + match w {
                            0x26 => (current + 4) % 5,
                            0x28 => (current + 1) % 5,
                            0x24 => 0,
                            _ => 4,
                        };
                } else if v.editing {
                    if w == 0x56 && GetKeyState(0x11) < 0 {
                        paste_minutes(hwnd);
                    } else if w == 0x41 && GetKeyState(0x11) < 0 {
                        view().select_all = true;
                    } else if w == 0x2e {
                        let mut current = view();
                        if current.select_all {
                            current.minutes.clear();
                            current.caret = 0;
                        } else if current.caret < current.minutes.len() {
                            let caret = current.caret;
                            current.minutes.remove(caret);
                        }
                        current.select_all = false;
                    } else if matches!(w, 0x25 | 0x27 | 0x24 | 0x23) {
                        let mut current = view();
                        current.caret = match w {
                            0x25 => current.caret.saturating_sub(1),
                            0x27 => (current.caret + 1).min(current.minutes.len()),
                            0x24 => 0,
                            _ => current.minutes.len(),
                        };
                        current.select_all = false;
                    } else if w == 0x0d {
                        activate(hwnd, Command::ApplySleep);
                    }
                } else if v.focus >= buttons.len() && v.focus < buttons.len() + sliders.len() {
                    let slider = &sliders[v.focus - buttons.len()];
                    if let Some(value) = slider.value {
                        let next = match w {
                            0x25 | 0x28 => value - 1,
                            0x26 | 0x27 => value + 1,
                            0x24 => slider.min,
                            0x23 => slider.max,
                            _ => value,
                        };
                        if next != value {
                            change_slider(slider.kind, next.clamp(slider.min, slider.max), &s);
                        }
                    }
                } else if (w == 0x20 || w == 0x0d) && v.focus < buttons.len() {
                    let button = buttons.into_iter().nth(v.focus).unwrap();
                    if button.enabled {
                        activate(hwnd, button.command);
                    }
                }
                accessibility::focus_changed(hwnd);
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_CHAR => {
                if view().editing {
                    let mut v = view();
                    if w == 8 || (48..=57).contains(&w) {
                        v.minutes_dirty = true;
                    }
                    if w == 8 {
                        if v.select_all {
                            v.minutes.clear();
                            v.caret = 0;
                        } else if v.caret > 0 {
                            let index = v.caret - 1;
                            v.minutes.remove(index);
                            v.caret = index;
                        }
                        v.select_all = false;
                    } else if (48..=57).contains(&w) {
                        if v.select_all {
                            v.minutes.clear();
                            v.caret = 0;
                            v.select_all = false;
                        }
                        if v.minutes.len() < 6 {
                            let caret = v.caret.min(v.minutes.len());
                            v.minutes.insert(caret, char::from_u32(w as u32).unwrap());
                            v.caret = caret + 1;
                        }
                        v.error = None;
                    }
                    drop(v);
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_PAINT => {
                let started = Instant::now();
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut r = RECT::default();
                GetClientRect(hwnd, &mut r);
                let (width, height) = (r.right.max(1), r.bottom.max(1));
                if ui(hwnd)
                    .buffer
                    .as_ref()
                    .is_none_or(|b| b.width != width || b.height != height)
                {
                    ui(hwnd).sidebar_frame = None;
                    ui(hwnd).buffer = Some(Surface::new(dc, width, height));
                }
                let mem = ui(hwnd).buffer.as_ref().unwrap().dc;
                let target = crate::render::target(mem, width, height, W, H);
                let s = state();
                let v = view().clone();
                let moving = sidebar_animating(&v) && !s.reduce_motion && !v.os_reduce_motion;
                let cached = moving
                    && ui(hwnd)
                        .sidebar_frame
                        .as_ref()
                        .is_some_and(|f| f.page == v.page && f.theme == s.theme);
                if cached {
                    let frame = ui(hwnd).sidebar_frame.as_ref().unwrap();
                    BitBlt(mem, 0, 0, width, height, frame.surface.dc, 0, 0, SRCCOPY);
                    crate::theme::set(s.theme);
                    fill(mem, rect(72, 53, W - 72, 678), crate::theme::current().bg);
                    let delta = (frame.shift - content_shift(&v)) * width / W;
                    let source = frame.left * width / W;
                    let top = 53 * height / H;
                    BitBlt(
                        mem,
                        source + delta,
                        top,
                        width - source,
                        678 * height / H,
                        frame.surface.dc,
                        source,
                        top,
                        SRCCOPY,
                    );
                    paint_chrome(mem, &s, &v, &controls(&s, &v));
                } else {
                    paint(mem, &s, false, 0, None, None, None);
                    ui(hwnd).sidebar_frame = None;
                    if moving {
                        let surface = Surface::new(dc, width, height);
                        BitBlt(surface.dc, 0, 0, width, height, mem, 0, 0, SRCCOPY);
                        ui(hwnd).sidebar_frame = Some(SidebarFrame {
                            surface,
                            shift: content_shift(&v),
                            left: sidebar_width(&v) + 1,
                            page: v.page,
                            theme: s.theme,
                        });
                    }
                }
                drop(target);
                BitBlt(dc, 0, 0, width, height, mem, 0, 0, SRCCOPY);
                EndPaint(hwnd, &ps);
                if cached {
                    let times = &mut ui(hwnd).frame_times;
                    times.push_back(started.elapsed().as_micros().min(u32::MAX as u128) as u32);
                    if times.len() > 120 {
                        times.pop_front();
                    }
                }
                if moving {
                    DwmFlush();
                }
                0
            }
            WM_ERASEBKGND => 1,
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_SIZE if w == SIZE_MINIMIZED as usize => 0,
            WM_DPICHANGED => {
                ui(hwnd).dpi = (w & 0xffff) as u32;
                let r = &*(l as *const RECT);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                InvalidateRect(hwnd, null(), 0);
                0
            }
            ICON_STATUS => ui(hwnd).icon_color as isize,
            PREVIEW_STATUS => PREVIEW_RESULT.load(Ordering::Relaxed) as isize,
            ANIMATION_STATUS => {
                let times = &ui(hwnd).frame_times;
                match w {
                    0 => times.len() as isize,
                    1 => {
                        times.iter().map(|v| *v as u64).sum::<u64>() as isize
                            / times.len().max(1) as isize
                    }
                    _ => times.iter().copied().max().unwrap_or(0) as isize,
                }
            }
            SHOW => {
                show(hwnd);
                animate(hwnd);
                0
            }
            WM_DESTROY => {
                animation_clock(hwnd, false);
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
                BeginPaint(hwnd, &mut ps);
                paint_preview(hwnd);
                EndPaint(hwnd, &ps);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
unsafe fn make_icon(blade_color: u32) -> HICON {
    unsafe {
        let dc = GetDC(null_mut());
        let mem = CreateCompatibleDC(dc);
        let bm = CreateCompatibleBitmap(dc, 64, 64);
        let old = SelectObject(mem, bm);
        let target = crate::render::target(mem, 64, 64, 32, 32);
        crate::render::fill(mem, rect(0, 0, 32, 32), 0x231b13);
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
        crate::render::polygon(mem, &blade, blade_color);
        let handle = [
            POINT { x: 21, y: 9 },
            POINT { x: 24, y: 11 },
            POINT { x: 11, y: 30 },
            POINT { x: 8, y: 28 },
        ];
        crate::render::polygon(mem, &handle, 0xf7f1e9);
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
unsafe fn render(
    path: &std::path::Path,
    compact: bool,
    scale: i32,
    about: bool,
) -> std::io::Result<()> {
    unsafe {
        let (width, height) = if about {
            about::SIZE
        } else if compact {
            (290i32, 100i32)
        } else {
            (W, H)
        };
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
        if about {
            crate::theme::set(state().theme);
            about::paint_preview(dc, scale);
        } else {
            paint(dc, &state(), compact, -1, None, None, None);
        }
        drop(target);
        GdiFlush();
        let pixels = std::slice::from_raw_parts_mut(bits, (width * height * 4) as usize);
        if compact {
            preview_alpha(pixels, width as usize, height as usize);
        }
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
    let _apartment = unsafe { accessibility::Apartment::new() };
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--sidebar-expanded")
        && args.iter().any(|a| a.starts_with("--render"))
    {
        let mut v = view();
        v.sidebar = 1.0;
        v.sidebar_open = true;
    }

    if let Some(index) = args.iter().position(|a| a == "--page")
        && let Some(page) = args.get(index + 1).and_then(|v| v.parse::<u8>().ok())
    {
        view().page = page.min(4);
    }
    if let Some(index) = args.iter().position(|a| a == "--theme")
        && let Some(theme) = args.get(index + 1).and_then(|v| v.parse::<u8>().ok())
    {
        state.lock().unwrap().theme = theme.min(4);
    }
    unsafe {
        refresh_motion_preference();
    }
    STATE.set(state).ok();
    REFRESH.set(refresh).ok();
    COMMANDS.set(commands).ok();
    unsafe {
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) == 0 {
            SetProcessDPIAware();
        }
        let args: Vec<String> = std::env::args().collect();
        if let Some(index) = args.iter().position(|arg| arg == "--render-demo") {
            let directory = args
                .get(index + 1)
                .map(std::path::Path::new)
                .unwrap_or_else(|| std::path::Path::new("scythe-demo"));
            if let Err(error) = demo::export(directory) {
                eprintln!("Demo render failed: {error}");
            }
            return;
        }
        if let Some(i) = args.iter().position(|a| {
            a == "--render" || a == "--render-hover" || a == "--render-2x" || a == "--render-about"
        }) {
            if args.iter().any(|a| a == "--charging-preview") {
                let mut s = STATE.get().unwrap().lock().unwrap();
                s.charging = Some(true);
                if s.battery.is_none() {
                    s.battery = Some(75);
                }
            }
            if let Some(index) = args.iter().position(|a| a == "--charge-phase")
                && let Some(phase) = args
                    .get(index + 1)
                    .and_then(|v| v.parse::<f32>().ok())
                    .filter(|v| v.is_finite())
            {
                let _ = PREVIEW_CHARGE_PHASE.set(phase.clamp(0., 1.));
            }
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
                args[i] == "--render-about",
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
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED,
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
        let icon_color = crate::battery::color(self::state().battery);
        let data = Box::new(Ui {
            tip,
            icon: make_icon(icon_color),
            icon_color,
            charging_animation: false,
            baseline: Some(self::state().mic_mute_notifications),
            drag: None,
            dpi,
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            buffer: None,
            sidebar_frame: None,
            animation_clock: false,
            animation_timer: null_mut(),
            frame_times: std::collections::VecDeque::new(),
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(data) as isize);
        paint_preview(tip);
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

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn supported() -> State {
        State {
            preset: Some(1),
            audio: crate::audio::AudioState {
                speaker: Some(75),
                microphone: Some(90),
                ..Default::default()
            },
            mic_eq: Some(32),
            sidetone: Some(5),
            game_chat_balance: Some(10),
            features: crate::features::Features {
                bands: Some([0; 10]),
                sleep: Some(37),
                anc: Some([1, 4]),
                wireless: Some(1),
                voice_prompt: Some(true),
                led: Some(0),
            },
            ..Default::default()
        }
    }
    fn overlaps(a: RECT, b: RECT) -> bool {
        a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
    }
    #[test]
    fn sidebar_animation_is_reversible_and_respects_motion_preferences() {
        assert_eq!(sidebar_progress(0.0, true, 0.0, false), 0.0);
        let middle = sidebar_progress(0.0, true, 0.08, false);
        assert!(middle > 0.0 && middle < 1.0);
        assert_eq!(sidebar_progress(middle, false, 0.0, false), middle);
        assert_eq!(sidebar_progress(0.0, true, 0.28, false), 1.0);
        assert_eq!(sidebar_progress(1.0, false, 0.28, false), 0.0);
        assert_eq!(sidebar_progress(0.0, true, 0.0, true), 1.0);
        let mut v = View {
            os_reduce_motion: true,
            ..View::default()
        };
        update_sidebar(&mut v, true, false);
        assert_eq!(sidebar_width(&v), 204);
        update_sidebar(&mut v, false, false);
        assert_eq!(sidebar_width(&v), 72);
    }
    #[test]
    fn menus_and_navigation_stay_available_in_both_sidebar_states() {
        let s = supported();
        for sidebar in [0.0, 0.4, 1.0] {
            let v = View {
                sidebar,
                ..View::default()
            };
            let buttons = controls(&s, &v);
            assert_eq!(
                buttons
                    .iter()
                    .filter(|b| matches!(b.command, Command::File | Command::Help))
                    .count(),
                2
            );
            assert_eq!(
                buttons
                    .iter()
                    .filter(|b| matches!(b.command, Command::Page(_)))
                    .count(),
                5
            );
            for b in buttons
                .iter()
                .filter(|b| matches!(b.command, Command::Page(_) | Command::Refresh))
            {
                assert!(b.r.right <= sidebar_width(&v));
                assert!(b.r.right - b.r.left >= 44 && b.r.bottom - b.r.top >= 44);
            }
        }
    }
    #[test]
    fn interactive_targets_fit_and_do_not_overlap_on_any_page() {
        for s in [State::default(), supported()] {
            for (page, sidebar) in
                (0..5).flat_map(|page| [0.0, 0.5, 1.0].map(|sidebar| (page, sidebar)))
            {
                let v = View {
                    page,
                    sidebar,
                    ..View::default()
                };
                let buttons = controls(&s, &v);
                let sliders = sliders(&s, &v);
                let targets: Vec<RECT> = buttons
                    .iter()
                    .map(|b| b.r)
                    .chain(sliders.iter().map(|sl| {
                        rect(
                            sl.r.left - 8,
                            sl.r.top - 5,
                            sl.r.right - sl.r.left + 16,
                            sl.r.bottom - sl.r.top + 10,
                        )
                    }))
                    .collect();
                for (index, r) in targets.iter().enumerate() {
                    assert!(
                        r.left >= 0 && r.top >= 0 && r.right <= W && r.bottom <= H,
                        "page {page} target {index} outside canvas"
                    );
                    for other in &targets[index + 1..] {
                        assert!(
                            !overlaps(*r, *other),
                            "overlapping interactive targets on page {page}: {index}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn sliders_map_pointer_positions_to_exact_endpoints() {
        let s = supported();
        for page in [0, 1, 2] {
            let v = View {
                page,
                ..View::default()
            };
            for slider in sliders(&s, &v) {
                if slider.vertical {
                    assert_eq!(
                        slider_value(&slider, slider.r.left, slider.r.top),
                        slider.max
                    );
                    assert_eq!(
                        slider_value(&slider, slider.r.left, slider.r.bottom),
                        slider.min
                    );
                } else {
                    assert_eq!(
                        slider_value(&slider, slider.r.left, slider.r.top),
                        slider.min
                    );
                    assert_eq!(
                        slider_value(&slider, slider.r.right, slider.r.top),
                        slider.max
                    );
                }
            }
        }
    }
    #[test]
    fn eq_apply_requires_an_available_matching_slot_and_staged_changes() {
        let s = supported();
        let mut v = View {
            page: 1,
            ..View::default()
        };
        let enabled = |v: &View| {
            controls(&s, v)
                .into_iter()
                .find(|b| matches!(b.command, Command::ApplyEq))
                .unwrap()
                .enabled
        };
        assert!(!enabled(&v));
        v.draft = Some([1; 10]);
        v.draft_slot = Some(1);
        assert!(enabled(&v));
        v.draft_slot = Some(2);
        assert!(!enabled(&v));
    }
    #[test]
    fn unknown_hardware_mute_is_not_reported_as_live() {
        let mut s = supported();
        s.hardware_mic_muted = None;
        assert_eq!(microphone_status(&s), "Headset status unavailable");
        s.hardware_mic_muted = Some(false);
        assert_eq!(microphone_status(&s), "Ready to speak");
        s.hardware_mic_muted = Some(true);
        assert_eq!(microphone_status(&s), "Muted on headset");
    }
    #[test]
    fn native_icon_blade_uses_battery_color_and_keeps_handle_neutral() {
        unsafe {
            for battery in [
                Some(100),
                Some(60),
                Some(59),
                Some(20),
                Some(19),
                Some(0),
                None,
            ] {
                let color = crate::battery::color(battery);
                let icon = make_icon(color);
                assert!(!icon.is_null());
                let mut info: ICONINFO = zeroed();
                assert_ne!(GetIconInfo(icon, &mut info), 0);
                let dc = CreateCompatibleDC(null_mut());
                let mut bitmap_info: BITMAPINFO = zeroed();
                bitmap_info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
                bitmap_info.bmiHeader.biWidth = 64;
                bitmap_info.bmiHeader.biHeight = -64;
                bitmap_info.bmiHeader.biPlanes = 1;
                bitmap_info.bmiHeader.biBitCount = 32;
                bitmap_info.bmiHeader.biCompression = BI_RGB;
                let mut pixels = vec![0u32; 64 * 64];
                assert_eq!(
                    GetDIBits(
                        dc,
                        info.hbmColor,
                        0,
                        64,
                        pixels.as_mut_ptr().cast(),
                        &mut bitmap_info,
                        DIB_RGB_COLORS
                    ),
                    64
                );
                let pixel_color = |c: u32| ((c & 255) << 16) | (c & 0xff00) | ((c >> 16) & 255);
                assert!(
                    pixels
                        .iter()
                        .filter(|p| **p & 0xffffff == pixel_color(color))
                        .count()
                        > 100
                );
                assert!(
                    pixels
                        .iter()
                        .filter(|p| **p & 0xffffff == pixel_color(0xf7f1e9))
                        .count()
                        > 70
                );
                DeleteDC(dc);
                DeleteObject(info.hbmColor);
                DeleteObject(info.hbmMask);
                DestroyIcon(icon);
            }
        }
    }
    #[test]
    fn preview_corners_are_transparent_and_edges_premultiplied_for_all_themes() {
        for scale in [1, 2] {
            for palette in crate::theme::PALETTES {
                let width = 290 * scale;
                let height = 100 * scale;
                let color = [
                    (palette.panel >> 16) as u8,
                    (palette.panel >> 8) as u8,
                    palette.panel as u8,
                    255,
                ];
                let mut pixels = vec![0u8; width * height * 4];
                for pixel in pixels.as_chunks_mut::<4>().0 {
                    pixel.copy_from_slice(&color);
                }
                preview_alpha(&mut pixels, width, height);
                for index in [0, width - 1, (height - 1) * width, height * width - 1] {
                    assert_eq!(&pixels[index * 4..index * 4 + 4], &[0, 0, 0, 0]);
                }
                let center = (height / 2 * width + width / 2) * 4;
                assert_eq!(&pixels[center..center + 4], &color);
                assert!(
                    pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|p| p[3] > 0 && p[3] < 255)
                );
                assert!(
                    pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .all(|p| p[..3].iter().all(|v| *v <= p[3]))
                );
            }
        }
    }
    #[test]
    fn charging_animation_requires_charging_and_respects_reduce_motion() {
        assert_eq!(charging_phase(false, false, 1.), None);
        assert_eq!(charging_phase(true, true, 1.), None);
        assert_eq!(charging_phase(true, false, 0.), Some(0.));
        assert!((charging_phase(true, false, 1.9).unwrap() - 0.5).abs() < 0.001);
        assert!((charging_phase(true, false, 3.8).unwrap()).abs() < 0.001);
    }
}
