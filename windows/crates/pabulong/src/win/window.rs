//! The "Pabulong" window: a sidebar (Calls, Settings, Diagnostics) and one
//! page at a time, built from standard Win32 controls. Labels match the
//! macOS window (`NoiselessMainWindow`, `AppSettingsView`, `DiagnosticsView`).

use super::ui::{self, Theme};
use super::{dialogs, h};
use crate::settings::{SuppressionMode, VoiceIsolation};
use crate::text;
use std::collections::HashMap;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, FillRect, InvalidateRect, SetBkMode, SetTextColor, HBRUSH, HDC, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, LoadCursorW, MoveWindow, RegisterClassExW, SendMessageW, SetForegroundWindow,
    ShowWindow, BM_SETCHECK, CB_ADDSTRING, CB_RESETCONTENT, CB_SETCURSEL, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, IDC_ARROW, LB_ADDSTRING,
    LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL, MINMAXINFO, SW_RESTORE, SW_SHOW, WINDOW_EX_STYLE, WM_GETMINMAXINFO, WNDCLASSEXW,
    WS_OVERLAPPEDWINDOW,
};

pub const MAIN_CLASS: &str = "PabulongMain";
pub const NOTICES_CLASS: &str = "PabulongNotices";

// Control ids.
pub const NAV_CALLS: i32 = 200;
pub const NAV_SETTINGS: i32 = 201;
pub const NAV_DIAGNOSTICS: i32 = 202;
pub const SIDEBAR_PROCESSING: i32 = 203;
pub const REC_BUTTON: i32 = 210;
pub const CALLS_LIST: i32 = 211;
pub const CALL_REVEAL: i32 = 212;
pub const CALL_DELETE: i32 = 213;
pub const S_PROCESSING: i32 = 300;
pub const S_MIC: i32 = 301;
pub const S_SUPPRESSION: i32 = 302;
pub const S_ECHO: i32 = 303;
pub const S_VOICES: i32 = 304;
pub const S_BYPASS: i32 = 305;
pub const S_AUTOREC: i32 = 306;
pub const S_REVIEW: i32 = 307;
pub const S_LOCALREC: i32 = 308;
pub const S_SYSAUDIO: i32 = 309;
pub const S_OPEN_STORAGE: i32 = 310;
pub const S_LOGIN: i32 = 311;
pub const S_PRIVACY: i32 = 312;
pub const S_TERMS: i32 = 313;
pub const S_NOTICES: i32 = 314;
pub const D_REFRESH: i32 = 400;
pub const D_EXPORT: i32 = 401;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Calls,
    Settings,
    Diagnostics,
}

/// What the window shows; built by `app` from the controller each tick.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewModel {
    pub status: String,
    pub processing_on: bool,
    // Recording strip
    pub recording_title: String,
    pub recording_subtitle: String,
    pub is_recording: bool,
    pub can_record: bool,
    // Calls
    pub calls: Vec<String>,
    // Settings
    pub devices: Vec<String>,
    pub selected_device: usize,
    pub suppression: SuppressionMode,
    pub echo: bool,
    pub voices: VoiceIsolation,
    pub bypass: bool,
    pub auto_recording: bool,
    pub consent_needed: bool,
    pub local_recording: bool,
    pub system_audio: bool,
    pub launch_at_login: bool,
    pub version: String,
    // Diagnostics: (title, value)
    pub tiles: Vec<(&'static str, String)>,
    pub health: String,
}

#[derive(Default)]
struct Settings {
    heading_audio: HWND,
    processing: HWND,
    mic_label: HWND,
    mic: HWND,
    suppression_label: HWND,
    suppression: HWND,
    echo: HWND,
    echo_caption: HWND,
    voices_label: HWND,
    voices: HWND,
    voices_caption: HWND,
    bypass: HWND,
    heading_system: HWND,
    login: HWND,
    heading_recording: HWND,
    auto_rec: HWND,
    paused: HWND,
    review: HWND,
    local_rec: HWND,
    sys_audio: HWND,
    sys_caption: HWND,
    open_storage: HWND,
    heading_about: HWND,
    version: HWND,
    copyright: HWND,
    privacy: HWND,
    terms: HWND,
    notices: HWND,
}

pub struct MainWindow {
    pub hwnd: HWND,
    page: Page,
    background: HBRUSH,
    sidebar_brush: HBRUSH,
    // Sidebar
    title: HWND,
    status: HWND,
    nav: [HWND; 3],
    side_processing: HWND,
    // Calls
    calls_heading: HWND,
    calls_count: HWND,
    rec_title: HWND,
    rec_subtitle: HWND,
    rec_button: HWND,
    calls_list: HWND,
    empty_title: HWND,
    empty_caption: HWND,
    reveal: HWND,
    delete: HWND,
    // Settings
    s: Settings,
    // Diagnostics
    diag_heading: HWND,
    diag_status: HWND,
    tiles: Vec<(HWND, HWND)>,
    health: HWND,
    refresh: HWND,
    export: HWND,
    // Styling
    headings: Vec<HWND>,
    captions: Vec<HWND>,
    warnings: Vec<HWND>,
    shown: Option<ViewModel>,
    texts: HashMap<isize, String>,
}

pub unsafe fn register_classes(instance: HINSTANCE, wndproc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT, notices_proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT) {
    let icon = super::icons::load(super::icons::APP, 32).unwrap_or_default();
    let small = super::icons::load(super::icons::APP, 16).unwrap_or_default();
    let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
    for (class, proc) in [(MAIN_CLASS, wndproc), (NOTICES_CLASS, notices_proc)] {
        let name = super::wide(class);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(proc),
            hInstance: instance,
            hIcon: icon,
            hCursor: cursor,
            lpszClassName: PCWSTR(name.as_ptr()),
            hIconSm: small,
            ..Default::default()
        };
        RegisterClassExW(&wc);
    }
}

fn scale(hwnd: HWND, px: i32) -> i32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as i32;
    px * dpi / 96
}

fn place(hwnd: HWND, x: i32, y: i32, w: i32, height: i32) {
    unsafe {
        let _ = MoveWindow(hwnd, x, y, w.max(0), height.max(0), true);
    }
}

impl MainWindow {
    pub fn create(instance: HINSTANCE, theme: &Theme) -> Option<MainWindow> {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                &h(MAIN_CLASS),
                &h(text::APP_NAME),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                980,
                680,
                None,
                None,
                Some(instance),
                None,
            )
            .ok()?
        };
        let (t, hd, b, c) = (theme.title, theme.heading, theme.body, theme.caption);
        let mut captions = Vec::new();
        let mut warnings = Vec::new();
        let heading = |s: &str| ui::label(hwnd, s, hd);

        let title = ui::label(hwnd, text::APP_NAME, t);
        let status = ui::label(hwnd, "", c);
        captions.push(status);
        let nav = [
            ui::button(hwnd, text::CALLS, NAV_CALLS, b),
            ui::button(hwnd, text::SETTINGS, NAV_SETTINGS, b),
            ui::button(hwnd, text::DIAGNOSTICS, NAV_DIAGNOSTICS, b),
        ];
        let side_processing = ui::checkbox(hwnd, text::MIC_PROCESSING, SIDEBAR_PROCESSING, b);

        // Calls
        let calls_heading = heading(text::CALLS);
        let calls_count = ui::label(hwnd, "", c);
        captions.push(calls_count);
        let rec_title = ui::label(hwnd, "", hd);
        let rec_subtitle = ui::label(hwnd, "", c);
        captions.push(rec_subtitle);
        let rec_button = ui::button(hwnd, text::RECORD, REC_BUTTON, b);
        let calls_list = ui::listbox(hwnd, CALLS_LIST, b);
        let empty_title = heading(text::NO_CALLS);
        let empty_caption = ui::label(hwnd, text::RECORDINGS_APPEAR_HERE, c);
        captions.push(empty_caption);
        let reveal = ui::button(hwnd, text::REVEAL, CALL_REVEAL, b);
        let delete = ui::button(hwnd, text::DELETE, CALL_DELETE, b);

        // Settings
        let mut s = Settings {
            heading_audio: heading(text::SECTION_AUDIO),
            heading_system: heading(text::SECTION_SYSTEM),
            heading_recording: heading(text::SECTION_RECORDING),
            heading_about: heading(text::SECTION_ABOUT),
            ..Settings::default()
        };
        s.processing = ui::checkbox(hwnd, text::MIC_PROCESSING, S_PROCESSING, b);
        s.mic_label = ui::label(hwnd, text::MICROPHONE, b);
        s.mic = ui::combo(hwnd, S_MIC, b);
        s.suppression_label = ui::label(hwnd, text::SUPPRESSION, b);
        s.suppression = ui::combo(hwnd, S_SUPPRESSION, b);
        for m in SuppressionMode::ALL {
            add_string(s.suppression, m.display_name());
        }
        s.echo = ui::checkbox(hwnd, text::ECHO_CANCELLATION, S_ECHO, b);
        s.echo_caption = ui::label(hwnd, text::ECHO_CANCELLATION_CAPTION, c);
        s.voices_label = ui::label(hwnd, text::BACKGROUND_VOICES, b);
        s.voices = ui::combo(hwnd, S_VOICES, b);
        for v in VoiceIsolation::ALL {
            add_string(s.voices, v.display_name());
        }
        s.voices_caption = ui::label(hwnd, text::BACKGROUND_VOICES_CAPTION, c);
        s.bypass = ui::checkbox(hwnd, text::BYPASS, S_BYPASS, b);
        s.login = ui::checkbox(hwnd, text::LAUNCH_AT_LOGIN, S_LOGIN, b);
        s.auto_rec = ui::checkbox(hwnd, text::AUTOMATIC_CALL_RECORDING, S_AUTOREC, b);
        s.paused = ui::label(hwnd, text::PAUSED_UNTIL_CONSENT, c);
        warnings.push(s.paused);
        s.review = ui::button(hwnd, text::REVIEW, S_REVIEW, b);
        s.local_rec = ui::checkbox(hwnd, text::LOCAL_RECORDING, S_LOCALREC, b);
        s.sys_audio = ui::checkbox(hwnd, text::CAPTURE_SYSTEM_AUDIO, S_SYSAUDIO, b);
        s.sys_caption = ui::label(hwnd, text::CAPTURE_SYSTEM_AUDIO_CAPTION, c);
        s.open_storage = ui::button(hwnd, text::OPEN_CALL_STORAGE, S_OPEN_STORAGE, b);
        s.version = ui::label(hwnd, "", b);
        s.copyright = ui::label(hwnd, text::COPYRIGHT, c);
        s.privacy = ui::button(hwnd, text::PRIVACY_NOTICE, S_PRIVACY, b);
        s.terms = ui::button(hwnd, text::TERMS_OF_USE, S_TERMS, b);
        s.notices = ui::button(hwnd, text::THIRD_PARTY_NOTICES, S_NOTICES, b);
        captions.extend([s.echo_caption, s.voices_caption, s.sys_caption, s.copyright]);

        // Diagnostics
        let diag_heading = heading(text::DIAGNOSTICS);
        let diag_status = ui::label(hwnd, "", c);
        captions.push(diag_status);
        let tiles: Vec<(HWND, HWND)> = (0..10)
            .map(|_| {
                let title = ui::label(hwnd, "", c);
                captions.push(title);
                (title, ui::label(hwnd, "", b))
            })
            .collect();
        let health = ui::label(hwnd, "", c);
        captions.push(health);
        let refresh = ui::button(hwnd, text::REFRESH, D_REFRESH, b);
        let export = ui::button(hwnd, text::EXPORT_DIAGNOSTICS, D_EXPORT, b);
        let headings = vec![
            title, calls_heading, empty_title, rec_title, s.heading_audio, s.heading_system, s.heading_recording, s.heading_about, diag_heading,
        ];

        let mut w = MainWindow {
            hwnd,
            page: Page::Settings,
            background: theme.background,
            sidebar_brush: theme.sidebar,
            title,
            status,
            nav,
            side_processing,
            calls_heading,
            calls_count,
            rec_title,
            rec_subtitle,
            rec_button,
            calls_list,
            empty_title,
            empty_caption,
            reveal,
            delete,
            s,
            diag_heading,
            diag_status,
            tiles,
            health,
            refresh,
            export,
            headings,
            captions,
            warnings,
            shown: None,
            texts: HashMap::new(),
        };
        w.set_page(Page::Settings);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
        Some(w)
    }

    pub fn bring_to_front(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    pub fn close(&self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn set_page(&mut self, page: Page) {
        self.page = page;
        let calls = [self.calls_heading, self.calls_count, self.rec_title, self.rec_subtitle, self.rec_button, self.calls_list, self.empty_title, self.empty_caption, self.reveal, self.delete];
        for c in calls {
            ui::show(c, page == Page::Calls);
        }
        for c in self.settings_controls() {
            ui::show(c, page == Page::Settings);
        }
        let mut diag = vec![self.diag_heading, self.diag_status, self.health, self.refresh, self.export];
        diag.extend(self.tiles.iter().flat_map(|(a, b)| [*a, *b]));
        for c in diag {
            ui::show(c, page == Page::Diagnostics);
        }
        // Re-apply conditional visibility within the page.
        if let Some(vm) = self.shown.take() {
            self.update(&vm);
        }
        self.layout();
    }

    fn settings_controls(&self) -> Vec<HWND> {
        let s = &self.s;
        vec![
            s.heading_audio, s.processing, s.mic_label, s.mic, s.suppression_label, s.suppression, s.echo, s.echo_caption, s.voices_label, s.voices,
            s.voices_caption, s.bypass, s.heading_system, s.login, s.heading_recording, s.auto_rec, s.paused, s.review, s.local_rec, s.sys_audio,
            s.sys_caption, s.open_storage, s.heading_about, s.version, s.copyright, s.privacy, s.terms, s.notices,
        ]
    }

    pub fn layout(&self) {
        let hwnd = self.hwnd;
        let px = |v: i32| scale(hwnd, v);
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
        }
        let (width, height) = (rc.right, rc.bottom);
        let side = px(220);
        let row = px(28);
        let gap = px(8);

        // Sidebar
        let mut y = px(20);
        place(self.title, px(20), y, side - px(40), px(34));
        y += px(36);
        place(self.status, px(20), y, side - px(40), px(20));
        y += px(36);
        for b in self.nav {
            place(b, px(16), y, side - px(32), px(34));
            y += px(40);
        }
        place(self.side_processing, px(20), height - px(48), side - px(40), row);

        let x0 = side + px(28);
        let content = width - x0 - px(28);
        match self.page {
            Page::Calls => {
                let mut y = px(20);
                place(self.calls_heading, x0, y, content, px(30));
                y += px(30);
                place(self.calls_count, x0, y, content, px(20));
                y += px(32);
                place(self.rec_title, x0, y, content - px(130), px(24));
                place(self.rec_button, x0 + content - px(120), y + px(4), px(120), px(36));
                y += px(24);
                place(self.rec_subtitle, x0, y, content - px(130), px(20));
                y += px(36);
                let list_h = height - y - px(70);
                place(self.calls_list, x0, y, content, list_h);
                place(self.empty_title, x0, y + px(40), content, px(30));
                place(self.empty_caption, x0, y + px(72), content, px(20));
                let by = y + list_h + px(14);
                place(self.reveal, x0, by, px(110), px(32));
                place(self.delete, x0 + px(120), by, px(110), px(32));
            }
            Page::Settings => {
                let col = (content - px(32)) / 2;
                let caption = |hw: HWND, y: &mut i32, lines: i32| {
                    place(hw, x0, *y, col, px(18) * lines);
                    *y += px(18) * lines + gap;
                };
                let s = &self.s;
                // Left column: Audio, System.
                let mut y = px(20);
                place(s.heading_audio, x0, y, col, px(30));
                y += px(36);
                place(s.processing, x0, y, col, row);
                y += row + gap;
                place(s.mic_label, x0, y, col, px(20));
                y += px(22);
                place(s.mic, x0, y, col, px(300));
                y += row + gap + px(4);
                place(s.suppression_label, x0, y, col, px(20));
                y += px(22);
                place(s.suppression, x0, y, col, px(200));
                y += row + gap + px(4);
                place(s.echo, x0, y, col, row);
                y += row;
                caption(s.echo_caption, &mut y, 2);
                place(s.voices_label, x0, y, col, px(20));
                y += px(22);
                place(s.voices, x0, y, col, px(200));
                y += row + px(4);
                caption(s.voices_caption, &mut y, 3);
                place(s.bypass, x0, y, col, row);
                y += row + px(20);
                place(s.heading_system, x0, y, col, px(30));
                y += px(36);
                place(s.login, x0, y, col, row);

                // Right column: Recording, About.
                let x1 = x0 + col + px(32);
                let mut y = px(20);
                place(s.heading_recording, x1, y, col, px(30));
                y += px(36);
                place(s.auto_rec, x1, y, col, row);
                y += row + gap;
                place(s.paused, x1, y, col - px(90), px(36));
                place(s.review, x1 + col - px(84), y, px(84), px(30));
                y += px(40);
                place(s.local_rec, x1, y, col, row);
                y += row + gap;
                place(s.sys_audio, x1, y, col, row);
                y += row;
                place(s.sys_caption, x1, y, col, px(36));
                y += px(36) + gap;
                place(s.open_storage, x1, y, px(180), px(32));
                y += px(32) + px(24);
                place(s.heading_about, x1, y, col, px(30));
                y += px(36);
                place(s.version, x1, y, col, px(22));
                y += px(26);
                place(s.copyright, x1, y, col, px(36));
                y += px(40);
                let bw = (col - px(16)) / 3;
                place(s.privacy, x1, y, bw, px(32));
                place(s.terms, x1 + bw + px(8), y, bw, px(32));
                place(s.notices, x1 + 2 * (bw + px(8)), y, bw, px(32));
            }
            Page::Diagnostics => {
                let mut y = px(20);
                place(self.diag_heading, x0, y, content, px(30));
                y += px(30);
                place(self.diag_status, x0, y, content, px(20));
                y += px(36);
                let col = (content - px(24)) / 2;
                for (i, (title, value)) in self.tiles.iter().enumerate() {
                    let x = if i % 2 == 0 { x0 } else { x0 + col + px(24) };
                    let ty = y + (i as i32 / 2) * px(62);
                    place(*title, x, ty, col, px(18));
                    place(*value, x, ty + px(20), col, px(26));
                }
                y += 5 * px(62) + px(8);
                place(self.health, x0, y, content, px(36));
                y += px(48);
                place(self.refresh, x0, y, px(120), px(32));
                place(self.export, x0 + px(130), y, px(170), px(32));
            }
        }
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }

    fn text(&mut self, hwnd: HWND, s: &str) {
        let key = hwnd.0 as isize;
        if self.texts.get(&key).map(String::as_str) != Some(s) {
            ui::set_text(hwnd, s);
            self.texts.insert(key, s.to_string());
        }
    }

    pub fn update(&mut self, vm: &ViewModel) {
        let prev = self.shown.take();
        let changed = |f: &dyn Fn(&ViewModel) -> bool| prev.as_ref().is_none_or(|p| f(p));

        self.text(self.status, &vm.status);
        check(self.side_processing, vm.processing_on);

        // Calls page
        self.text(self.calls_count, &format!("{} {}", vm.calls.len(), text::SAVED_SUFFIX));
        self.text(self.rec_title, &vm.recording_title);
        self.text(self.rec_subtitle, &vm.recording_subtitle);
        self.text(self.rec_button, if vm.is_recording { text::STOP } else { text::RECORD });
        enable(self.rec_button, vm.is_recording || vm.can_record);
        if changed(&|p| p.calls != vm.calls) {
            let selected = unsafe { SendMessageW(self.calls_list, LB_GETCURSEL, None, None) }.0;
            unsafe {
                SendMessageW(self.calls_list, LB_RESETCONTENT, None, None);
            }
            for c in &vm.calls {
                let w = super::wide(c);
                unsafe {
                    SendMessageW(self.calls_list, LB_ADDSTRING, None, Some(LPARAM(w.as_ptr() as isize)));
                }
            }
            if selected >= 0 && (selected as usize) < vm.calls.len() {
                unsafe {
                    SendMessageW(self.calls_list, LB_SETCURSEL, Some(WPARAM(selected as usize)), None);
                }
            }
        }
        let on_calls = self.page == Page::Calls;
        let empty = vm.calls.is_empty();
        ui::show(self.calls_list, on_calls && !empty);
        ui::show(self.empty_title, on_calls && empty);
        ui::show(self.empty_caption, on_calls && empty);
        enable(self.reveal, !empty);
        enable(self.delete, !empty);

        // Settings page
        let s = &self.s;
        check(s.processing, vm.processing_on);
        if changed(&|p| p.devices != vm.devices) {
            unsafe {
                SendMessageW(s.mic, CB_RESETCONTENT, None, None);
            }
            for d in &vm.devices {
                add_string(s.mic, d);
            }
        }
        select(s.mic, vm.selected_device);
        select(s.suppression, SuppressionMode::ALL.iter().position(|m| *m == vm.suppression).unwrap_or(1));
        check(s.echo, vm.echo);
        select(s.voices, VoiceIsolation::ALL.iter().position(|v| *v == vm.voices).unwrap_or(2));
        check(s.bypass, vm.bypass);
        check(s.login, vm.launch_at_login);
        check(s.auto_rec, vm.auto_recording);
        check(s.local_rec, vm.local_recording);
        check(s.sys_audio, vm.system_audio);
        let on_settings = self.page == Page::Settings;
        ui::show(s.paused, on_settings && vm.auto_recording && vm.consent_needed);
        ui::show(s.review, on_settings && vm.auto_recording && vm.consent_needed);
        ui::show(s.sys_caption, on_settings && vm.system_audio);
        let version = s.version;
        self.text(version, &format!("{}    {}", text::VERSION, vm.version));

        // Diagnostics page
        let status = vm.status.clone();
        self.text(self.diag_status, &status);
        for (i, (title, value)) in vm.tiles.iter().enumerate() {
            if let Some(&(t, v)) = self.tiles.get(i) {
                self.text(t, title);
                self.text(v, value);
            }
        }
        let health = vm.health.clone();
        self.text(self.health, &health);

        self.shown = Some(vm.clone());
    }

    pub fn selected_call(&self) -> Option<usize> {
        let i = unsafe { SendMessageW(self.calls_list, LB_GETCURSEL, None, None) }.0;
        (i >= 0).then_some(i as usize)
    }

    /// WM_CTLCOLORSTATIC / WM_CTLCOLORBTN: brand colours on the right background.
    pub fn color(&self, dc: HDC, control: HWND) -> LRESULT {
        let color: COLORREF = if self.headings.contains(&control) {
            ui::NAVY
        } else if self.warnings.contains(&control) {
            ui::WARNING
        } else if self.captions.contains(&control) {
            ui::SECONDARY
        } else {
            ui::NAVY
        };
        let in_sidebar = [self.title, self.status, self.side_processing].contains(&control) || self.nav.contains(&control);
        unsafe {
            SetTextColor(dc, color);
            SetBkMode(dc, TRANSPARENT);
        }
        LRESULT(if in_sidebar { self.sidebar_brush.0 as isize } else { self.background.0 as isize })
    }

    pub fn paint(&self) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(self.hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            FillRect(dc, &rc, self.background);
            let side = RECT { right: scale(self.hwnd, 220), ..rc };
            FillRect(dc, &side, self.sidebar_brush);
            let _ = EndPaint(self.hwnd, &ps);
        }
    }
}

fn add_string(combo: HWND, s: &str) {
    let w = super::wide(s);
    unsafe {
        SendMessageW(combo, CB_ADDSTRING, None, Some(LPARAM(w.as_ptr() as isize)));
    }
}

fn select(combo: HWND, index: usize) {
    unsafe {
        SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(index)), None);
    }
}

fn check(button: HWND, on: bool) {
    unsafe {
        SendMessageW(button, BM_SETCHECK, Some(WPARAM(usize::from(on))), None);
    }
}

fn enable(hwnd: HWND, on: bool) {
    unsafe {
        let _ = EnableWindow(hwnd, on);
    }
}

/// Messages of the main window that need no app state.
pub unsafe fn default_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_GETMINMAXINFO => {
            let info = lparam.0 as *mut MINMAXINFO;
            if !info.is_null() {
                (*info).ptMinTrackSize.x = scale(hwnd, 900);
                (*info).ptMinTrackSize.y = scale(hwnd, 640);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ------------------------------------------------------- notices window

pub struct NoticesWindow {
    pub hwnd: HWND,
    heading: HWND,
    body: HWND,
    done: HWND,
}

impl NoticesWindow {
    pub fn create(instance: HINSTANCE, theme: &Theme, owner: Option<HWND>) -> Option<Self> {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                &h(NOTICES_CLASS),
                &h(text::THIRD_PARTY_NOTICES),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                600,
                560,
                owner,
                None,
                Some(instance),
                None,
            )
            .ok()?
        };
        let (heading, body, done) = dialogs::notices_contents(hwnd, theme);
        let w = Self { hwnd, heading, body, done };
        w.layout();
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        Some(w)
    }

    pub fn layout(&self) {
        let px = |v: i32| scale(self.hwnd, v);
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        place(self.heading, px(20), px(16), rc.right - px(40), px(30));
        place(self.body, px(20), px(52), rc.right - px(40), rc.bottom - px(112));
        place(self.done, rc.right - px(120), rc.bottom - px(48), px(100), px(32));
    }

    pub fn is(&self, hwnd: HWND) -> bool {
        self.hwnd == hwnd
    }
}

pub const fn command_id(wparam: WPARAM) -> i32 {
    (wparam.0 & 0xFFFF) as i32
}

pub const fn command_code(wparam: WPARAM) -> u32 {
    ((wparam.0 >> 16) & 0xFFFF) as u32
}
