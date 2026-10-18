//! The notification-area icon and its menu — the Windows version of the
//! macOS menu bar popover (`MenuBarPopover`).

use super::{copy_to_fixed, icons};
use crate::text;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW,
    NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, GetSystemMetrics, SetForegroundWindow, TrackPopupMenu, HICON,
    HMENU, MENU_ITEM_FLAGS, MF_CHECKED, MF_DEFAULT, MF_GRAYED, MF_SEPARATOR, MF_STRING, SM_CXSMICON, TPM_BOTTOMALIGN, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP,
};

/// Callback message for tray events.
pub const WM_TRAY: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;

pub const CMD_TOGGLE_PROCESSING: usize = 10;
pub const CMD_RECORD: usize = 11;
pub const CMD_MIC_SETTINGS: usize = 12;
pub const CMD_FIX_DEFAULT: usize = 13;
pub const CMD_OPEN: usize = 14;
pub const CMD_QUIT: usize = 15;

pub struct Tray {
    hwnd: HWND,
    icon: Option<HICON>,
    shown_on: Option<bool>,
    tip: String,
}

/// Everything the menu shows, computed from the controller.
pub struct MenuState {
    pub subtitle: String,
    pub processing_on: bool,
    pub show_recording: bool,
    pub recording_title: String,
    pub recording_subtitle: String,
    pub is_recording: bool,
    pub can_record: bool,
    pub mic_access_denied: bool,
    pub virtual_mic_is_default: bool,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Self {
        Self { hwnd, icon: None, shown_on: None, tip: String::new() }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAY,
            hIcon: self.icon.unwrap_or_default(),
            ..Default::default()
        };
        copy_to_fixed(&self.tip, &mut nid.szTip);
        nid
    }

    /// Adds the icon (again, after Explorer restarts).
    pub fn add(&mut self, on: bool, tip: &str) {
        self.set_icon(on);
        self.tip = tip.to_string();
        let mut nid = self.data();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
            nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
        }
    }

    fn set_icon(&mut self, on: bool) {
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
        if let Some(old) = self.icon.take() {
            unsafe {
                let _ = DestroyIcon(old);
            }
        }
        self.icon = icons::tray(on, size);
        self.shown_on = Some(on);
    }

    /// Updates the icon (processing on/off) and tooltip when they change.
    pub fn update(&mut self, on: bool, tip: &str, force: bool) {
        if !force && self.shown_on == Some(on) && self.tip == tip {
            return;
        }
        self.set_icon(on);
        self.tip = tip.to_string();
        let nid = self.data();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    pub fn remove(&mut self) {
        let nid = self.data();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
        if let Some(icon) = self.icon.take() {
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
    }

    /// Shows the menu at the cursor and returns the chosen command (0 = none).
    /// Runs a modal loop: call it without holding app state borrowed.
    pub fn show_menu(hwnd: HWND, state: &MenuState) -> usize {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else { return 0 };
            build(menu, state);
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            // Required so the menu closes when the user clicks elsewhere.
            let _ = SetForegroundWindow(hwnd);
            let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, hwnd, None);
            let _ = DestroyMenu(menu);
            cmd.0 as usize
        }
    }
}

unsafe fn item(menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, label: &str) {
    let w = super::wide(label);
    let _ = AppendMenuW(menu, MF_STRING | flags, id, PCWSTR(w.as_ptr()));
}

unsafe fn separator(menu: HMENU) {
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
}

unsafe fn build(menu: HMENU, s: &MenuState) {
    // Header: "Pabulong" and its status line, like the popover's title row.
    item(menu, MF_GRAYED, 0, &format!("{} — {}", text::APP_NAME, s.subtitle));
    separator(menu);
    item(menu, if s.processing_on { MF_CHECKED } else { MENU_ITEM_FLAGS(0) }, CMD_TOGGLE_PROCESSING, text::MIC_PROCESSING);

    if s.show_recording {
        separator(menu);
        item(menu, MF_GRAYED, 0, &format!("{} · {}", s.recording_title, s.recording_subtitle));
        let (label, enabled) = if s.is_recording { (text::STOP_RECORDING_HELP, true) } else { (text::START_RECORDING_HELP, s.can_record) };
        item(menu, if enabled { MENU_ITEM_FLAGS(0) } else { MF_GRAYED }, CMD_RECORD, label);
    }

    if s.processing_on && s.mic_access_denied {
        item(menu, MENU_ITEM_FLAGS(0), CMD_MIC_SETTINGS, &format!("⚠ {}", text::MICROPHONE_SETTINGS));
    }

    if s.virtual_mic_is_default {
        separator(menu);
        item(menu, MF_GRAYED, 0, &format!("⚠ {}", text::DEFAULT_INPUT_WARNING));
        item(menu, MF_GRAYED, 0, text::DEFAULT_INPUT_DETAIL);
        item(menu, MENU_ITEM_FLAGS(0), CMD_FIX_DEFAULT, text::DEFAULT_INPUT_FIX);
    }

    separator(menu);
    item(menu, MF_DEFAULT, CMD_OPEN, text::OPEN_PABULONG);
    item(menu, MENU_ITEM_FLAGS(0), CMD_QUIT, text::QUIT);
}
