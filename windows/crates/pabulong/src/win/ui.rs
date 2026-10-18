//! Brand fonts and colours, and small helpers for standard Win32 controls.

use super::{h, wide};
use std::ffi::c_void;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AddFontMemResourceEx, CreateFontW, CreateSolidBrush, GetDC, GetTextFaceW, ReleaseDC, SelectObject, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, FW_NORMAL, FW_SEMIBOLD, HBRUSH, HFONT, OUT_TT_PRECIS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, SendMessageW, SetWindowTextW, ShowWindow, BS_AUTOCHECKBOX, BS_PUSHBUTTON, CBS_DROPDOWNLIST, ES_AUTOVSCROLL, ES_MULTILINE,
    ES_READONLY, HMENU, LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, SW_HIDE, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_SETFONT, WS_CHILD,
    WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

// Kit colours.
pub const NAVY: COLORREF = rgb(0x0D, 0x2C, 0x52);
pub const SIGNAL_BLUE: COLORREF = rgb(0x1A, 0x3C, 0xFF);
pub const OFF_WHITE: COLORREF = rgb(0xF7, 0xF7, 0xF8);
pub const WHITE: COLORREF = rgb(0xFF, 0xFF, 0xFF);
pub const SECONDARY: COLORREF = rgb(0x5B, 0x64, 0x74);
pub const WARNING: COLORREF = rgb(0xB4, 0x5A, 0x00);

const MONTSERRAT: &[u8] = include_bytes!("../../../../assets/fonts/Montserrat-wght.ttf");
const DM_SANS: &[u8] = include_bytes!("../../../../assets/fonts/DMSans-wght.ttf");

pub struct Theme {
    pub title: HFONT,
    pub heading: HFONT,
    pub body: HFONT,
    pub caption: HFONT,
    pub background: HBRUSH,
    pub sidebar: HBRUSH,
}

/// Loads the brand fonts privately (this process only) and falls back to
/// Segoe UI if GDI does not expose the expected family.
pub fn theme(dpi: u32) -> Theme {
    unsafe {
        for data in [MONTSERRAT, DM_SANS] {
            let mut installed = 0u32;
            AddFontMemResourceEx(data.as_ptr() as *const c_void, data.len() as u32, None, &mut installed);
        }
    }
    let px = |pt: i32| -(pt * dpi as i32 / 72);
    Theme {
        title: font("Montserrat", px(16), FW_SEMIBOLD.0 as i32),
        heading: font("Montserrat", px(11), FW_SEMIBOLD.0 as i32),
        body: font("DM Sans", px(10), FW_NORMAL.0 as i32),
        caption: font("DM Sans", px(9), FW_NORMAL.0 as i32),
        background: unsafe { CreateSolidBrush(OFF_WHITE) },
        sidebar: unsafe { CreateSolidBrush(WHITE) },
    }
}

fn create(face: &str, height: i32, weight: i32) -> HFONT {
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            &h(face),
        )
    }
}

fn face_of(font: HFONT) -> String {
    unsafe {
        let dc = GetDC(None);
        let old = SelectObject(dc, font.into());
        let mut buf = [0u16; 64];
        let n = GetTextFaceW(dc, Some(&mut buf));
        SelectObject(dc, old);
        ReleaseDC(None, dc);
        String::from_utf16_lossy(&buf[..(n.max(1) as usize - 1)])
    }
}

fn font(face: &str, height: i32, weight: i32) -> HFONT {
    let f = create(face, height, weight);
    if face_of(f).starts_with(face) {
        f
    } else {
        create("Segoe UI", height, weight)
    }
}

pub fn set_font(hwnd: HWND, font: HFONT) {
    unsafe {
        SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    }
}

fn child(parent: HWND, class: &str, text: &str, style: WINDOW_STYLE, id: i32, font: HFONT) -> HWND {
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            &h(class),
            &h(text),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            10,
            10,
            Some(parent),
            Some(HMENU(id as isize as *mut c_void)),
            None,
            None,
        )
        .unwrap_or_default()
    };
    set_font(hwnd, font);
    hwnd
}

pub fn label(parent: HWND, text: &str, font: HFONT) -> HWND {
    child(parent, "STATIC", text, WINDOW_STYLE(0), 0, font) // SS_LEFT
}

pub fn checkbox(parent: HWND, text: &str, id: i32, font: HFONT) -> HWND {
    child(parent, "BUTTON", text, WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32), id, font)
}

pub fn button(parent: HWND, text: &str, id: i32, font: HFONT) -> HWND {
    child(parent, "BUTTON", text, WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), id, font)
}

pub fn combo(parent: HWND, id: i32, font: HFONT) -> HWND {
    child(parent, "COMBOBOX", "", WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32), id, font)
}

pub fn listbox(parent: HWND, id: i32, font: HFONT) -> HWND {
    child(parent, "LISTBOX", "", WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE((LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32), id, font)
}

pub fn readonly_text(parent: HWND, text: &str, font: HFONT) -> HWND {
    child(parent, "EDIT", text, WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32), 0, font)
}

pub fn set_text(hwnd: HWND, text: &str) {
    unsafe {
        let _ = SetWindowTextW(hwnd, &h(text));
    }
}

pub fn show(hwnd: HWND, visible: bool) {
    unsafe {
        let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
    }
}

/// Keeps `buf` alive while a PCWSTR to it is in use.
pub fn pcw(buf: &[u16]) -> PCWSTR {
    PCWSTR(buf.as_ptr())
}

pub fn wstr(s: &str) -> Vec<u16> {
    wide(s)
}
