//! Scythe-themed modal dialog with native accessible controls and app rendering.
#![allow(unsafe_op_in_unsafe_fn)]
use super::*;
use windows_sys::Win32::UI::{Controls::*, Input::KeyboardAndMouse::*};
const WIDTH: i32 = 500;
const HEIGHT: i32 = 416;
const PROJECT: i32 = 100;
const CLOSE: i32 = 1;
const DISMISS: i32 = 2;
struct Dialog {
    dpi: u32,
}
unsafe fn data(hwnd: HWND) -> &'static mut Dialog {
    &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Dialog)
}
#[derive(Clone)]
struct Element {
    id: i32,
    label: &'static str,
    bounds: RECT,
    size: i32,
    bold: bool,
    muted: bool,
    button: bool,
}
fn elements() -> Vec<Element> {
    let text = |id, label, bounds, size, bold, muted| Element {
        id,
        label,
        bounds,
        size,
        bold,
        muted,
        button: false,
    };
    vec![
        text(101, "ABOUT SCYTHE", rect(24, 15, 380, 24), 11, true, true),
        text(102, "Scythe", rect(96, 73, 350, 43), 31, true, false),
        text(
            103,
            concat!("Version ", env!("CARGO_PKG_VERSION"), "  /  Windows x64"),
            rect(97, 116, 350, 23),
            12,
            false,
            true,
        ),
        text(
            104,
            "Control for your BlackShark V3 Pro.",
            rect(28, 172, 444, 24),
            15,
            false,
            false,
        ),
        text(
            105,
            "Offline. No account or telemetry.",
            rect(28, 203, 444, 24),
            13,
            false,
            true,
        ),
        text(
            106,
            "Copyright 2026 havioo  /  MIT License",
            rect(44, 254, 410, 22),
            12,
            false,
            false,
        ),
        text(
            107,
            "Clear Sans  /  Apache License 2.0",
            rect(44, 279, 410, 22),
            12,
            false,
            true,
        ),
        text(
            108,
            "Independent project. Not affiliated with Razer.",
            rect(28, 324, 444, 22),
            11,
            false,
            true,
        ),
        Element {
            id: PROJECT,
            label: "GitHub project",
            bounds: rect(28, 366, 140, 34),
            size: 12,
            bold: false,
            muted: false,
            button: true,
        },
        Element {
            id: CLOSE,
            label: "Close",
            bounds: rect(372, 366, 100, 34),
            size: 12,
            bold: true,
            muted: false,
            button: true,
        },
        Element {
            id: DISMISS,
            label: "Close About Scythe",
            bounds: rect(444, 10, 32, 32),
            size: 14,
            bold: false,
            muted: true,
            button: true,
        },
    ]
}
fn template() -> Vec<u32> {
    // Borderless modal: Windows manages focus and accessibility; Scythe draws it.
    let style = WS_POPUP | WS_CLIPCHILDREN;
    let mut words = vec![
        style as u16,
        (style >> 16) as u16,
        0,
        0,
        0,
        0,
        0,
        300,
        240,
        0,
        0,
    ];
    words.extend(wide("About Scythe"));
    if words.len() % 2 != 0 {
        words.push(0);
    }
    words
        .as_chunks::<2>()
        .0
        .iter()
        .map(|w| w[0] as u32 | ((w[1] as u32) << 16))
        .collect()
}
unsafe fn draw_element(dc: HDC, element: &Element, focused: bool, hovered: bool, pressed: bool) {
    let p = crate::theme::current();
    let width = element.bounds.right - element.bounds.left;
    let height = element.bounds.bottom - element.bounds.top;
    let bounds = rect(0, 0, width, height);
    let panel = matches!(element.id, 106 | 107);
    crate::render::clear(dc, if panel { p.panel } else { p.bg });
    if !element.button {
        crate::render::text(
            dc,
            bounds,
            element.label,
            if element.muted { p.muted } else { p.text },
            element.size,
            if element.bold { 600 } else { 400 },
            false,
        );
        return;
    }
    if focused {
        crate::render::rounded(dc, bounds, p.text, 10);
    }
    let inset = if focused { 2 } else { 0 };
    let bounds = rect(inset, inset, width - 2 * inset, height - 2 * inset);
    let primary = element.id == CLOSE;
    let background = if primary {
        p.accent
    } else if hovered || pressed {
        mix(p.panel, p.text, 0.10)
    } else if element.id == DISMISS {
        p.bg
    } else {
        p.panel
    };
    crate::render::rounded(
        dc,
        bounds,
        if pressed {
            mix(background, p.text, 0.10)
        } else {
            background
        },
        8,
    );
    let ink = if primary {
        crate::theme::on_accent()
    } else if element.muted {
        p.muted
    } else {
        p.text
    };
    if element.id == DISMISS {
        crate::render::line(
            dc,
            &[POINT { x: 11, y: 11 }, POINT { x: 21, y: 21 }],
            ink,
            1.3,
        );
        crate::render::line(
            dc,
            &[POINT { x: 21, y: 11 }, POINT { x: 11, y: 21 }],
            ink,
            1.3,
        );
    } else {
        crate::render::text(
            dc,
            rect(16, 0, width - 28, height),
            element.label,
            ink,
            element.size,
            if element.bold { 600 } else { 400 },
            false,
        );
    }
}
unsafe fn draw_background(dc: HDC) {
    let p = crate::theme::current();
    crate::render::clear(dc, p.bg);
    crate::render::fill(dc, rect(1, 51, WIDTH - 2, 1), mix(p.bg, p.text, 0.10));
    crate::render::rounded(dc, rect(28, 246, 444, 62), p.panel, 12);
    let blade = [
        (6, 18),
        (13, 7),
        (27, 4),
        (40, 10),
        (45, 21),
        (34, 15),
        (22, 13),
        (12, 18),
    ]
    .map(|(x, y)| POINT {
        x: 28 + x,
        y: 84 + y,
    });
    let handle = [(31, 13), (36, 16), (16, 45), (12, 42)].map(|(x, y)| POINT {
        x: 28 + x,
        y: 84 + y,
    });
    crate::render::polygon(dc, &blade, p.accent);
    crate::render::polygon(dc, &handle, p.text);
}
unsafe extern "system" fn button(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        BM_SETSTYLE => {
            // Dialog focus management can otherwise turn an owner-drawn button
            // into a standard Windows default button.
            return DefSubclassProc(hwnd, msg, (w & !0xf) | BS_OWNERDRAW as usize, l);
        }
        WM_MOUSEMOVE => {
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut tracking);
            InvalidateRect(hwnd, null(), 0);
        }
        WM_MOUSELEAVE => {
            InvalidateRect(hwnd, null(), 0);
        }
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(null_mut(), IDC_HAND));
            return 1;
        }
        WM_NCDESTROY => {
            RemoveWindowSubclass(hwnd, Some(button), 1);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, w, l)
}
unsafe extern "system" fn label_control(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            let mut bounds = RECT::default();
            GetClientRect(hwnd, &mut bounds);
            if let Some(element) = elements().iter().find(|e| e.id == GetDlgCtrlID(hwnd)) {
                let surface = Surface::new(dc, bounds.right, bounds.bottom);
                let _target = crate::render::target(
                    surface.dc,
                    bounds.right,
                    bounds.bottom,
                    element.bounds.right - element.bounds.left,
                    element.bounds.bottom - element.bounds.top,
                );
                draw_element(surface.dc, element, false, false, false);
                BitBlt(
                    dc,
                    0,
                    0,
                    bounds.right,
                    bounds.bottom,
                    surface.dc,
                    0,
                    0,
                    SRCCOPY,
                );
            }
            EndPaint(hwnd, &ps);
            return 0;
        }
        WM_ERASEBKGND => return 1,
        WM_NCDESTROY => {
            RemoveWindowSubclass(hwnd, Some(label_control), 1);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, w, l)
}
unsafe fn layout(hwnd: HWND, dpi: u32) {
    data(hwnd).dpi = dpi;
    let scale = |value: i32| value * dpi as i32 / 96;
    for e in elements() {
        let existing = GetDlgItem(hwnd, e.id);
        if existing.is_null() {
            let control = CreateWindowExW(
                0,
                wide(if e.button { "BUTTON" } else { "STATIC" }).as_ptr(),
                wide(e.label).as_ptr(),
                WS_CHILD
                    | WS_VISIBLE
                    | if e.button {
                        WS_TABSTOP | BS_OWNERDRAW as u32
                    } else {
                        0
                    },
                scale(e.bounds.left),
                scale(e.bounds.top),
                scale(e.bounds.right - e.bounds.left),
                scale(e.bounds.bottom - e.bounds.top),
                hwnd,
                e.id as usize as HMENU,
                GetModuleHandleW(null()),
                null(),
            );
            if e.button {
                SetWindowSubclass(control, Some(button), 1, 0);
            } else {
                SetWindowSubclass(control, Some(label_control), 1, 0);
            }
        } else {
            SetWindowPos(
                existing,
                null_mut(),
                scale(e.bounds.left),
                scale(e.bounds.top),
                scale(e.bounds.right - e.bounds.left),
                scale(e.bounds.bottom - e.bounds.top),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}
unsafe extern "system" fn dialog(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            let owner = GetWindow(hwnd, GW_OWNER);
            let dpi = GetDpiForWindow(owner).max(96);
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                Box::into_raw(Box::new(Dialog { dpi })) as isize,
            );
            let width = WIDTH * dpi as i32 / 96;
            let height = HEIGHT * dpi as i32 / 96;
            let mut parent = RECT::default();
            GetWindowRect(owner, &mut parent);
            SetWindowPos(
                hwnd,
                null_mut(),
                parent.left + (parent.right - parent.left - width) / 2,
                parent.top + (parent.bottom - parent.top - height) / 2,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let corners = 2i32;
            DwmSetWindowAttribute(hwnd, 33, (&corners as *const i32).cast(), 4);
            layout(hwnd, dpi);
            super::SetFocus(GetDlgItem(hwnd, CLOSE));
            0
        }
        WM_COMMAND => {
            match w as u16 as i32 {
                CLOSE | DISMISS => {
                    EndDialog(hwnd, 0);
                }
                PROJECT => {
                    ShellExecuteW(
                        hwnd,
                        wide("open").as_ptr(),
                        wide("https://github.com/havioo/scythe").as_ptr(),
                        null(),
                        null(),
                        SW_SHOWNORMAL,
                    );
                }
                _ => return 0,
            }
            1
        }
        WM_CLOSE => {
            EndDialog(hwnd, 0);
            1
        }
        WM_NCHITTEST => {
            let mut point = POINT {
                x: l as i16 as i32,
                y: (l >> 16) as i16 as i32,
            };
            ScreenToClient(hwnd, &mut point);
            let dpi = data(hwnd).dpi as i32;
            if point.y < 52 * dpi / 96 && point.x < 430 * dpi / 96 {
                SetWindowLongPtrW(hwnd, DWLP_MSGRESULT as i32, HTCAPTION as isize);
                1
            } else {
                0
            }
        }
        WM_DRAWITEM => {
            let item = &*(l as *const DRAWITEMSTRUCT);
            if let Some(e) = elements().iter().find(|e| e.id == item.CtlID as i32) {
                let width = item.rcItem.right - item.rcItem.left;
                let height = item.rcItem.bottom - item.rcItem.top;
                let surface = Surface::new(item.hDC, width, height);
                let _target = crate::render::target(
                    surface.dc,
                    width,
                    height,
                    e.bounds.right - e.bounds.left,
                    e.bounds.bottom - e.bounds.top,
                );
                let mut cursor = POINT::default();
                GetCursorPos(&mut cursor);
                ScreenToClient(item.hwndItem, &mut cursor);
                draw_element(
                    surface.dc,
                    e,
                    item.itemState & ODS_FOCUS != 0 && item.itemState & ODS_NOFOCUSRECT == 0,
                    PtInRect(&item.rcItem, cursor) != 0,
                    item.itemState & ODS_SELECTED != 0,
                );
                BitBlt(item.hDC, 0, 0, width, height, surface.dc, 0, 0, SRCCOPY);
            }
            1
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            let mut r = RECT::default();
            GetClientRect(hwnd, &mut r);
            let surface = Surface::new(dc, r.right, r.bottom);
            let _target = crate::render::target(surface.dc, r.right, r.bottom, WIDTH, HEIGHT);
            draw_background(surface.dc);
            BitBlt(dc, 0, 0, r.right, r.bottom, surface.dc, 0, 0, SRCCOPY);
            EndPaint(hwnd, &ps);
            1
        }
        WM_ERASEBKGND => 1,
        WM_DPICHANGED => {
            let r = *(l as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            layout(hwnd, (w & 0xffff) as u32);
            InvalidateRect(hwnd, null(), 0);
            1
        }
        WM_NCDESTROY => {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Dialog;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !pointer.is_null() {
                drop(Box::from_raw(pointer));
            }
            0
        }
        _ => 0,
    }
}
pub(super) unsafe fn show(owner: HWND) {
    let template = template();
    DialogBoxIndirectParamW(
        GetModuleHandleW(null()),
        template.as_ptr().cast(),
        owner,
        Some(dialog),
        0,
    );
}

// Uses the same element painters as the modal for offline visual QA.
pub(super) unsafe fn paint_preview(dc: HDC, scale: i32) {
    draw_background(dc);
    for element in elements() {
        let width = element.bounds.right - element.bounds.left;
        let height = element.bounds.bottom - element.bounds.top;
        let surface = Surface::new(dc, width * scale, height * scale);
        let _target =
            crate::render::target(surface.dc, width * scale, height * scale, width, height);
        draw_element(surface.dc, &element, false, false, false);
        BitBlt(
            dc,
            element.bounds.left * scale,
            element.bounds.top * scale,
            width * scale,
            height * scale,
            surface.dc,
            0,
            0,
            SRCCOPY,
        );
    }
}
pub(super) const SIZE: (i32, i32) = (WIDTH, HEIGHT);
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn about_controls_have_names_and_do_not_overlap() {
        let entries = elements();
        for (index, e) in entries.iter().enumerate() {
            assert!(!e.label.is_empty());
            assert!(
                e.bounds.left >= 0
                    && e.bounds.top >= 0
                    && e.bounds.right <= WIDTH
                    && e.bounds.bottom <= HEIGHT
            );
            for other in &entries[index + 1..] {
                assert!(
                    !(e.bounds.left < other.bounds.right
                        && e.bounds.right > other.bounds.left
                        && e.bounds.top < other.bounds.bottom
                        && e.bounds.bottom > other.bounds.top)
                );
            }
        }
        assert_eq!(entries.iter().filter(|e| e.button).count(), 3);
    }
}
