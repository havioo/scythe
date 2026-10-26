//! Microsoft Active Accessibility bridge for the custom-drawn dashboard.
//! COM calls are marshaled to the window's STA. Child IDs are one-based and
//! follow the same control order as keyboard focus; no HWND is owned here.
#![allow(unsafe_op_in_unsafe_fn)]
use super::*;
use std::{ffi::c_void, sync::atomic::AtomicU32};
use windows_sys::{
    Win32::{System::Variant::VARIANT, UI::Accessibility::*},
    core::{BSTR, GUID, HRESULT},
};
const ACCESSIBLE: GUID = GUID::from_u128(0x618736e0_3c3d_11cf_810c_00aa00389b71);
const DISPATCH: GUID = GUID::from_u128(0x00020400_0000_0000_c000_000000000046);
const UNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const INVALID: HRESULT = 0x80070057u32 as i32;
const NOT_IMPL: HRESULT = 0x80004001u32 as i32;
const NO_INTERFACE: HRESULT = 0x80004002u32 as i32;
type Object = *mut Accessible;
type StringProperty = unsafe extern "system" fn(Object, VARIANT, *mut BSTR) -> HRESULT;
type VariantProperty = unsafe extern "system" fn(Object, VARIANT, *mut VARIANT) -> HRESULT;
#[repr(C)]
struct Vtable {
    query: unsafe extern "system" fn(Object, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(Object) -> u32,
    release: unsafe extern "system" fn(Object) -> u32,
    type_count: unsafe extern "system" fn(Object, *mut u32) -> HRESULT,
    type_info: unsafe extern "system" fn(Object, u32, u32, *mut *mut c_void) -> HRESULT,
    ids: unsafe extern "system" fn(
        Object,
        *const GUID,
        *mut *mut u16,
        u32,
        u32,
        *mut i32,
    ) -> HRESULT,
    invoke: unsafe extern "system" fn(
        Object,
        i32,
        *const GUID,
        u32,
        u16,
        *mut c_void,
        *mut VARIANT,
        *mut c_void,
        *mut u32,
    ) -> HRESULT,
    parent: unsafe extern "system" fn(Object, *mut *mut c_void) -> HRESULT,
    count: unsafe extern "system" fn(Object, *mut i32) -> HRESULT,
    child: unsafe extern "system" fn(Object, VARIANT, *mut *mut c_void) -> HRESULT,
    name: StringProperty,
    value: StringProperty,
    description: StringProperty,
    role: VariantProperty,
    state: VariantProperty,
    help: StringProperty,
    help_topic: unsafe extern "system" fn(Object, *mut BSTR, VARIANT, *mut i32) -> HRESULT,
    shortcut: StringProperty,
    focus: unsafe extern "system" fn(Object, *mut VARIANT) -> HRESULT,
    selection: unsafe extern "system" fn(Object, *mut VARIANT) -> HRESULT,
    action_name: StringProperty,
    select: unsafe extern "system" fn(Object, i32, VARIANT) -> HRESULT,
    location: unsafe extern "system" fn(
        Object,
        *mut i32,
        *mut i32,
        *mut i32,
        *mut i32,
        VARIANT,
    ) -> HRESULT,
    navigate: unsafe extern "system" fn(Object, i32, VARIANT, *mut VARIANT) -> HRESULT,
    hit_test: unsafe extern "system" fn(Object, i32, i32, *mut VARIANT) -> HRESULT,
    action: unsafe extern "system" fn(Object, VARIANT) -> HRESULT,
    put_name: unsafe extern "system" fn(Object, VARIANT, BSTR) -> HRESULT,
    put_value: unsafe extern "system" fn(Object, VARIANT, BSTR) -> HRESULT,
}
#[repr(C)]
struct Accessible {
    vtable: &'static Vtable,
    refs: AtomicU32,
    hwnd: HWND,
}
struct Node {
    name: String,
    value: String,
    description: String,
    shortcut: String,
    role: i32,
    state: i32,
    bounds: RECT,
}
fn nodes(s: &State, v: &View) -> Vec<Node> {
    let buttons = controls(s, v);
    let mut result: Vec<Node> = buttons
        .iter()
        .enumerate()
        .map(|(index, b)| {
            let (name, role, shortcut) = match b.command {
                Command::Page(page) => (b.label.clone(), 0x25, format!("F{}", page + 1)),
                Command::Hide => ("Hide to tray".into(), 0x2b, "Escape".into()),
                Command::Minimize => ("Minimize window".into(), 0x2b, String::new()),
                Command::SleepEdit => ("Custom sleep minutes".into(), 0x2a, String::new()),
                Command::File => ("File".into(), 0x0c, "Alt+F".into()),
                Command::Help => ("Help".into(), 0x0c, "Alt+H".into()),
                _ => (b.label.clone(), 0x2b, String::new()),
            };
            let description = match b.command {
                Command::SleepEdit => "Whole minutes from 1 to 255. Enter applies the timer.",
                Command::ApplyEq => {
                    "Writes staged bands to the current headset slot. Other slots are preserved."
                }
                Command::Page(_) => "Use Up or Down to move between sections, then Enter to open.",
                _ if !b.enabled => "Unavailable with the current device state.",
                _ => "Press Enter or Space to activate.",
            }
            .into();
            Node {
                name,
                value: if matches!(b.command, Command::SleepEdit) {
                    v.minutes.clone()
                } else {
                    String::new()
                },
                description,
                shortcut,
                role,
                state: 0x100000
                    | if !b.enabled { 1 } else { 0 }
                    | if b.selected {
                        if matches!(b.command, Command::Page(_)) {
                            2
                        } else {
                            8
                        }
                    } else {
                        0
                    }
                    | if v.focus == index { 4 } else { 0 },
                bounds: b.r,
            }
        })
        .collect();
    for (index, slider) in sliders(s, v).iter().enumerate() {
        let name = match slider.kind {
            SliderKind::Game => "Game output volume".into(),
            SliderKind::Mic => "Microphone volume".into(),
            SliderKind::Balance => "Game and Chat balance".into(),
            SliderKind::Sidetone => "Sidetone level".into(),
            SliderKind::Eq(band) => format!("Equalizer {} Hz", crate::features::FREQUENCIES[band]),
        };
        result.push(Node {
            name,
            value: slider
                .value
                .map_or_else(|| "Unavailable".into(), |value| format!("{value}")),
            description: format!(
                "Range {} to {}. Arrow keys adjust; Home and End select endpoints.",
                slider.min, slider.max
            ),
            shortcut: String::new(),
            role: 0x33,
            state: 0x100000
                | if slider.value.is_none() { 1 } else { 0 }
                | if v.focus == buttons.len() + index {
                    4
                } else {
                    0
                },
            bounds: rect(
                slider.r.left - 8,
                slider.r.top - 5,
                slider.r.right - slider.r.left + 16,
                slider.r.bottom - slider.r.top + 10,
            ),
        });
    }
    result.push(Node {
        name: "Headset status".into(),
        value: format!(
            "Battery {}. {}. {}",
            percent(s.battery),
            if s.charging == Some(true) {
                "Charging"
            } else {
                "Not charging"
            },
            microphone_status(s)
        ),
        description: s.status.clone(),
        shortcut: String::new(),
        role: 0x29,
        state: 0x40,
        bounds: rect(236 - content_shift(v), 85, 772, 75),
    });
    result
}
fn id(v: VARIANT) -> Result<i32, HRESULT> {
    unsafe {
        if v.Anonymous.Anonymous.vt == 3 && v.Anonymous.Anonymous.Anonymous.lVal >= 0 {
            Ok(v.Anonymous.Anonymous.Anonymous.lVal)
        } else {
            Err(INVALID)
        }
    }
}
fn integer(value: i32) -> VARIANT {
    let mut v = VARIANT::default();
    v.Anonymous.Anonymous.vt = 3;
    v.Anonymous.Anonymous.Anonymous.lVal = value;
    v
}
unsafe fn snapshot(this: Object) -> Result<(State, View), HRESULT> {
    if this.is_null() || IsWindow((*this).hwnd) == 0 {
        Err(0x800401fdu32 as i32)
    } else {
        Ok((super::state(), view().clone()))
    }
}
unsafe fn node(this: Object, child: VARIANT) -> Result<Option<Node>, HRESULT> {
    let child = id(child)?;
    let (s, v) = snapshot(this)?;
    if child == 0 {
        Ok(None)
    } else {
        nodes(&s, &v)
            .into_iter()
            .nth(child as usize - 1)
            .map(Some)
            .ok_or(INVALID)
    }
}
unsafe fn string(out: *mut BSTR, value: &str) -> HRESULT {
    if out.is_null() {
        return INVALID;
    }
    *out = null_mut();
    if value.is_empty() {
        return 1;
    }
    *out = SysAllocString(wide(value).as_ptr());
    if (*out).is_null() {
        0x8007000eu32 as i32
    } else {
        0
    }
}
unsafe extern "system" fn query(this: Object, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || iid.is_null() {
        return INVALID;
    }
    *out = null_mut();
    if [ACCESSIBLE, DISPATCH, UNKNOWN].iter().any(|g| {
        (*iid).data1 == g.data1
            && (*iid).data2 == g.data2
            && (*iid).data3 == g.data3
            && (*iid).data4 == g.data4
    }) {
        *out = this.cast();
        add_ref(this);
        0
    } else {
        NO_INTERFACE
    }
}
unsafe extern "system" fn add_ref(this: Object) -> u32 {
    (*this).refs.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "system" fn release(this: Object) -> u32 {
    let count = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if count == 0 {
        drop(Box::from_raw(this));
    }
    count
}
unsafe extern "system" fn type_count(_: Object, out: *mut u32) -> HRESULT {
    if out.is_null() {
        return INVALID;
    }
    *out = 0;
    0
}
unsafe extern "system" fn type_info(_: Object, _: u32, _: u32, out: *mut *mut c_void) -> HRESULT {
    if !out.is_null() {
        *out = null_mut();
    }
    NOT_IMPL
}
unsafe extern "system" fn ids(
    _: Object,
    _: *const GUID,
    _: *mut *mut u16,
    _: u32,
    _: u32,
    _: *mut i32,
) -> HRESULT {
    NOT_IMPL
}
unsafe extern "system" fn invoke(
    _: Object,
    _: i32,
    _: *const GUID,
    _: u32,
    _: u16,
    _: *mut c_void,
    _: *mut VARIANT,
    _: *mut c_void,
    _: *mut u32,
) -> HRESULT {
    NOT_IMPL
}
unsafe extern "system" fn parent(this: Object, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return INVALID;
    }
    *out = null_mut();
    AccessibleObjectFromWindow((*this).hwnd, OBJID_WINDOW as u32, &DISPATCH, out)
}
unsafe extern "system" fn count(this: Object, out: *mut i32) -> HRESULT {
    if out.is_null() {
        return INVALID;
    }
    *out = 0;
    match snapshot(this) {
        Ok((s, v)) => {
            *out = nodes(&s, &v).len() as i32;
            0
        }
        Err(e) => e,
    }
}
unsafe extern "system" fn child(this: Object, v: VARIANT, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return INVALID;
    }
    *out = null_mut();
    match node(this, v) {
        Ok(_) => 1,
        Err(e) => e,
    }
}
macro_rules! property {
    ($fn:ident,$field:ident,$root:expr) => {
        unsafe extern "system" fn $fn(this: Object, v: VARIANT, out: *mut BSTR) -> HRESULT {
            match node(this, v) {
                Ok(Some(n)) => string(out, &n.$field),
                Ok(None) => string(out, $root),
                Err(e) => e,
            }
        }
    };
}
property!(name, name, "Scythe headset controls");
property!(value, value, "");
property!(
    description,
    description,
    "Local headset controls. Tab moves focus; F1 to F5 opens sections."
);
property!(shortcut, shortcut, "");
unsafe extern "system" fn role(this: Object, v: VARIANT, out: *mut VARIANT) -> HRESULT {
    if out.is_null() {
        return INVALID;
    }
    *out = VARIANT::default();
    match node(this, v) {
        Ok(n) => {
            *out = integer(n.map_or(0x10, |n| n.role));
            0
        }
        Err(e) => e,
    }
}
unsafe extern "system" fn state(this: Object, v: VARIANT, out: *mut VARIANT) -> HRESULT {
    if out.is_null() {
        return INVALID;
    }
    *out = VARIANT::default();
    match node(this, v) {
        Ok(n) => {
            let mut flags = n.map_or(0, |n| n.state);
            if GetFocus() != (*this).hwnd {
                flags &= !4;
            }
            if IsWindowVisible((*this).hwnd) == 0 {
                flags |= 0x8000;
            }
            *out = integer(flags);
            0
        }
        Err(e) => e,
    }
}
unsafe extern "system" fn help(_: Object, _: VARIANT, out: *mut BSTR) -> HRESULT {
    string(out, "")
}
unsafe extern "system" fn help_topic(
    _: Object,
    out: *mut BSTR,
    _: VARIANT,
    topic: *mut i32,
) -> HRESULT {
    if !out.is_null() {
        *out = null_mut();
    }
    if !topic.is_null() {
        *topic = 0;
    }
    1
}
unsafe extern "system" fn focus(this: Object, out: *mut VARIANT) -> HRESULT {
    if out.is_null() {
        return INVALID;
    }
    *out = VARIANT::default();
    if GetFocus() != (*this).hwnd {
        return 1;
    }
    match snapshot(this) {
        Ok((s, v)) => {
            *out = integer(
                if v.focus < controls(&s, &v).len() + sliders(&s, &v).len() {
                    v.focus as i32 + 1
                } else {
                    0
                },
            );
            0
        }
        Err(e) => e,
    }
}
unsafe extern "system" fn selection(this: Object, out: *mut VARIANT) -> HRESULT {
    if out.is_null() {
        return INVALID;
    }
    *out = VARIANT::default();
    match snapshot(this) {
        Ok((s, v)) => {
            if let Some(i) = controls(&s, &v)
                .iter()
                .position(|b| matches!(b.command, Command::Page(_)) && b.selected)
            {
                *out = integer(i as i32 + 1);
                0
            } else {
                1
            }
        }
        Err(e) => e,
    }
}
unsafe extern "system" fn action_name(this: Object, v: VARIANT, out: *mut BSTR) -> HRESULT {
    match node(this, v) {
        Ok(Some(n)) if n.role == 0x2b || n.role == 0x25 || n.role == 0x0c => string(out, "Press"),
        Ok(_) => string(out, ""),
        Err(e) => e,
    }
}
unsafe extern "system" fn select(this: Object, flags: i32, v: VARIANT) -> HRESULT {
    if flags != 1 {
        return INVALID;
    }
    let child = match id(v) {
        Ok(v) if v > 0 => v as usize - 1,
        _ => return INVALID,
    };
    let (s, mut current) = match snapshot(this) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let entries = nodes(&s, &current);
    if child >= entries.len()
        || entries[child].state & 1 != 0
        || entries[child].state & 0x100000 == 0
    {
        return INVALID;
    }
    current.focus = child;
    current.keyboard_focus = true;
    current.editing = controls(&s, &current)
        .get(child)
        .is_some_and(|b| matches!(b.command, Command::SleepEdit));
    if current.editing {
        current.select_all = true;
        current.caret = current.minutes.len();
    }

    current.nav_keyboard = child < controls(&s, &current).len()
        && matches!(
            controls(&s, &current)[child].command,
            Command::Page(_) | Command::Refresh
        );
    let open = current.nav_keyboard;
    update_sidebar(&mut current, open, s.reduce_motion);
    *view() = current;
    SetFocus((*this).hwnd);
    SetTimer((*this).hwnd, 5, 16, None);
    InvalidateRect((*this).hwnd, null(), 0);
    focus_changed((*this).hwnd);
    0
}
unsafe extern "system" fn location(
    this: Object,
    x: *mut i32,
    y: *mut i32,
    width: *mut i32,
    height: *mut i32,
    v: VARIANT,
) -> HRESULT {
    if x.is_null() || y.is_null() || width.is_null() || height.is_null() {
        return INVALID;
    }
    *x = 0;
    *y = 0;
    *width = 0;
    *height = 0;
    let n = match node(this, v) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let mut client = RECT::default();
    GetClientRect((*this).hwnd, &mut client);
    let r = n.map_or(rect(0, 0, W, H), |n| n.bounds);
    let mut origin = POINT {
        x: r.left * client.right / W,
        y: r.top * client.bottom / H,
    };
    ClientToScreen((*this).hwnd, &mut origin);
    *x = origin.x;
    *y = origin.y;
    *width = (r.right - r.left) * client.right / W;
    *height = (r.bottom - r.top) * client.bottom / H;
    0
}
unsafe extern "system" fn navigate(
    this: Object,
    direction: i32,
    v: VARIANT,
    out: *mut VARIANT,
) -> HRESULT {
    if out.is_null() {
        return INVALID;
    }
    *out = VARIANT::default();
    let from = match id(v) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mut total = 0;
    let hr = count(this, &mut total);
    if hr != 0 {
        return hr;
    }
    let next = match direction {
        5 => from + 1,
        6 => from - 1,
        7 if from == 0 => 1,
        8 if from == 0 => total,
        _ => return NOT_IMPL,
    };
    if next < 1 || next > total {
        return 1;
    }
    *out = integer(next);
    0
}
unsafe extern "system" fn hit_test(this: Object, x: i32, y: i32, out: *mut VARIANT) -> HRESULT {
    if out.is_null() {
        return INVALID;
    }
    *out = VARIANT::default();
    let (s, v) = match snapshot(this) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mut p = POINT { x, y };
    ScreenToClient((*this).hwnd, &mut p);
    let mut client = RECT::default();
    GetClientRect((*this).hwnd, &mut client);
    let (x, y) = (
        p.x * W / client.right.max(1),
        p.y * H / client.bottom.max(1),
    );
    if !(0..W).contains(&x) || !(0..H).contains(&y) {
        return 1;
    }
    let found = nodes(&s, &v).iter().position(|n| contains(n.bounds, x, y));
    *out = integer(found.map_or(0, |i| i as i32 + 1));
    0
}
unsafe extern "system" fn action(this: Object, v: VARIANT) -> HRESULT {
    let child = match id(v) {
        Ok(v) if v > 0 => v as usize - 1,
        _ => return INVALID,
    };
    let (s, current) = match snapshot(this) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match controls(&s, &current).into_iter().nth(child) {
        Some(b) if b.enabled => {
            PostMessageW((*this).hwnd, WM_APP + 20, child, current.page as isize);
            0
        }
        _ => INVALID,
    }
}
unsafe extern "system" fn put_name(_: Object, _: VARIANT, _: BSTR) -> HRESULT {
    NOT_IMPL
}
unsafe extern "system" fn put_value(this: Object, v: VARIANT, text: BSTR) -> HRESULT {
    if text.is_null() {
        return INVALID;
    }
    let child = match id(v) {
        Ok(v) if v > 0 => v as usize - 1,
        _ => return INVALID,
    };
    let (s, current) = match snapshot(this) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let len = SysStringLen(text) as usize;
    if len > 16 {
        return INVALID;
    }
    let text = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
    let buttons = controls(&s, &current);
    if let Some(b) = buttons.get(child) {
        if !b.enabled || !matches!(b.command, Command::SleepEdit) {
            return INVALID;
        }
        let minutes = match crate::features::parse_minutes(&text) {
            Ok(v) => v,
            Err(_) => return INVALID,
        };
        let mut v = view();
        v.minutes = minutes.to_string();
        v.minutes_dirty = true;
        v.caret = v.minutes.len();
        v.select_all = false;
        drop(v);
        InvalidateRect((*this).hwnd, null(), 0);
        return 0;
    }
    if let Some(slider) = sliders(&s, &current).get(child - buttons.len()) {
        let parsed = match text.parse::<i32>() {
            Ok(v) if v >= slider.min && v <= slider.max => v,
            _ => return INVALID,
        };
        if slider.value.is_none() {
            return INVALID;
        }
        change_slider(slider.kind, parsed, &s);
        InvalidateRect((*this).hwnd, null(), 0);
        return 0;
    }
    INVALID
}
static VTABLE: Vtable = Vtable {
    query,
    add_ref,
    release,
    type_count,
    type_info,
    ids,
    invoke,
    parent,
    count,
    child,
    name,
    value,
    description,
    role,
    state,
    help,
    help_topic,
    shortcut,
    focus,
    selection,
    action_name,
    select,
    location,
    navigate,
    hit_test,
    action,
    put_name,
    put_value,
};
pub(super) unsafe fn object(hwnd: HWND, w: WPARAM) -> LRESULT {
    let this = Box::into_raw(Box::new(Accessible {
        vtable: &VTABLE,
        refs: AtomicU32::new(1),
        hwnd,
    }));
    let result = LresultFromObject(&ACCESSIBLE, w, this.cast());
    release(this);
    result
}
pub(super) unsafe fn focus_changed(hwnd: HWND) {
    let s = super::state();
    let v = view().clone();
    let total = controls(&s, &v).len() + sliders(&s, &v).len();
    NotifyWinEvent(
        EVENT_OBJECT_FOCUS,
        hwnd,
        OBJID_CLIENT,
        if v.focus < total {
            v.focus as i32 + 1
        } else {
            0
        },
    );
}
pub(super) unsafe fn page_changed(hwnd: HWND) {
    NotifyWinEvent(EVENT_OBJECT_REORDER, hwnd, OBJID_CLIENT, 0);
    NotifyWinEvent(EVENT_OBJECT_NAMECHANGE, hwnd, OBJID_CLIENT, 0);
}
/// Balance the UI thread's COM apartment even when window creation fails.
pub(super) struct Apartment(bool);
impl Apartment {
    pub(super) unsafe fn new() -> Self {
        use windows_sys::Win32::System::Com::*;
        Self(CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) >= 0)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                windows_sys::Win32::System::Com::CoUninitialize();
            }
        }
    }
}

/// Emit changes only when a semantic value changes, never on animation frames.
pub(super) unsafe fn poll(hwnd: HWND) {
    type Semantic = (String, String, i32);
    static PREVIOUS: OnceLock<Mutex<Vec<Semantic>>> = OnceLock::new();
    let s = super::state();
    let v = view().clone();
    let current: Vec<Semantic> = nodes(&s, &v)
        .into_iter()
        .map(|n| (n.name, n.value, n.state & !4))
        .collect();
    let changes = {
        let mut previous = PREVIOUS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap();
        let changes: Vec<(u32, i32)> = current
            .iter()
            .enumerate()
            .flat_map(|(index, entry)| {
                let mut events = Vec::new();
                if let Some(old) = previous.get(index) {
                    if old.0 != entry.0 {
                        events.push((EVENT_OBJECT_NAMECHANGE, index as i32 + 1));
                    }
                    if old.1 != entry.1 {
                        events.push((EVENT_OBJECT_VALUECHANGE, index as i32 + 1));
                    }
                    if old.2 != entry.2 {
                        events.push((EVENT_OBJECT_STATECHANGE, index as i32 + 1));
                    }
                }
                events
            })
            .collect();
        *previous = current;
        changes
    };
    for (event, child) in changes {
        NotifyWinEvent(event, hwnd, OBJID_CLIENT, child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semantic_controls_have_names_values_and_states() {
        for page in 0..5 {
            let s = super::super::tests::supported();
            let v = View {
                page,
                ..View::default()
            };
            let entries = nodes(&s, &v);
            assert!(entries.iter().all(|n| !n.name.is_empty()));
            assert_eq!(entries.iter().filter(|n| n.role == 0x25).count(), 5);
            assert!(
                entries
                    .iter()
                    .filter(|n| n.role == 0x33)
                    .all(|n| !n.value.is_empty())
            );
            assert_eq!(
                entries
                    .iter()
                    .find(|n| n.name == "Hide to tray")
                    .unwrap()
                    .shortcut,
                "Escape"
            );
        }
        let s = State::default();
        let v = View::default();
        assert!(
            nodes(&s, &v)
                .iter()
                .filter(|n| n.role == 0x33)
                .all(|n| n.state & 1 != 0)
        );
    }
    #[test]
    fn com_variant_and_vtable_match_windows_abi() {
        assert_eq!(size_of::<VARIANT>(), 24);
        assert_eq!(size_of::<Vtable>(), 28 * size_of::<usize>());
        assert_eq!(id(integer(7)), Ok(7));
        assert_eq!(id(VARIANT::default()), Err(INVALID));
        assert_eq!(id(integer(-1)), Err(INVALID));
    }
}
