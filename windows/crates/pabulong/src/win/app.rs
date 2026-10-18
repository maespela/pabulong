//! Startup, the message loop and event handling.
//!
//! App state lives in a thread-local on the UI thread. Modal loops (menus,
//! task dialogs) dispatch messages back into our window procedures, so state
//! is never kept borrowed across one: handlers return an `Action` and the
//! modal part runs after the borrow ends.

use super::devices::{self, InputDevice};
use super::engine::WinPlatform;
use super::tray::{self, MenuState, Tray};
use super::ui::{self, Theme};
use super::window::{self, MainWindow, NoticesWindow, Page, ViewModel};
use super::{dialogs, h, shell};
use crate::controller::{diagnostics_json, duration_text, Controller};
use crate::recording::{unix_now, CallSession, CallStore};
use crate::settings::{SettingsStore, SuppressionMode, VoiceIsolation};
use crate::text;
use std::cell::RefCell;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Controls::{InitCommonControlsEx, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, GetMessageW, IsDialogMessageW, KillTimer, PostMessageW, PostQuitMessage,
    RegisterClassExW, RegisterWindowMessageW, SendMessageW, SetTimer, TranslateMessage, CBN_SELCHANGE, CB_GETCURSEL, LBN_DBLCLK, MSG,
    WM_APP, WS_EX_TOOLWINDOW, WS_POPUP, WM_CLOSE, WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLORSTATIC, WM_DESTROY, WM_ENDSESSION, WM_LBUTTONUP,
    WM_PAINT, WM_QUERYENDSESSION, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_SIZE, WM_TIMER, WNDCLASSEXW,
};

const TRAY_CLASS: &str = "PabulongTray";
const TICK_TIMER: usize = 1;
const TICK_MS: u32 = 50;
/// Sent by a second launch to the running instance: open the window.
const WM_SHOW_MAIN: u32 = WM_APP + 2;

struct App {
    instance: HINSTANCE,
    controller: Controller,
    platform: WinPlatform,
    settings_store: SettingsStore,
    tray_hwnd: HWND,
    tray: Tray,
    theme: Theme,
    main: Option<MainWindow>,
    notices: Option<NoticesWindow>,
    devices: Vec<InputDevice>,
    calls: Vec<CallSession>,
    virtual_mic_default: bool,
    last_slow: Instant,
    last_calls: Instant,
    taskbar_created: u32,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` with the app state if it is available (not mid-borrow).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| cell.try_borrow_mut().ok().and_then(|mut guard| guard.as_mut().map(f)))
}

/// Work that needs a modal loop, done after the state borrow ends.
enum Action {
    None,
    ShowMenu(MenuState),
    Consent(AfterConsent),
    ConfirmDelete(usize, String),
    OpenNotices,
    Quit,
}

#[derive(Clone, Copy)]
enum AfterConsent {
    EnableAutoRecording,
    StartManualRecording,
    Review,
}

pub fn run() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // One instance per user session; a second launch opens the window.
        let _mutex = CreateMutexW(None, true, &h("Local\\Pabulong.TrayApp"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(existing) = FindWindowW(&h(TRAY_CLASS), windows::core::PCWSTR::null()) {
                let _ = PostMessageW(Some(existing), WM_SHOW_MAIN, WPARAM(0), LPARAM(0));
            }
            return;
        }
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let icc = INITCOMMONCONTROLSEX { dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_STANDARD_CLASSES };
        let _ = InitCommonControlsEx(&icc);
        let _ = SetCurrentProcessExplicitAppUserModelID(&h("NextMatterPro.Pabulong"));
    }

    let settings_store = SettingsStore::new(shell::settings_dir().join("settings.json"));
    let mut settings = settings_store.load();

    // First-run agreement: nothing runs until the user agrees.
    if settings.accepted_terms_version < text::CURRENT_TERMS_VERSION {
        if !dialogs::terms_agreement() {
            return;
        }
        settings.accepted_terms_version = text::CURRENT_TERMS_VERSION;
        let _ = settings_store.save(&settings);
    }

    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }.map(Into::into).unwrap_or_default();
    unsafe {
        let name = super::wide(TRAY_CLASS);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(tray_proc),
            hInstance: instance,
            lpszClassName: windows::core::PCWSTR(name.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        window::register_classes(instance, main_proc, notices_proc);
    }
    let Ok(tray_hwnd) = (unsafe {
        // A hidden top-level window (not message-only): it must receive the
        // TaskbarCreated and WM_SETTINGCHANGE broadcasts.
        CreateWindowExW(WS_EX_TOOLWINDOW, &h(TRAY_CLASS), &h(text::APP_NAME), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance), None)
    }) else {
        return;
    };
    let taskbar_created = unsafe { RegisterWindowMessageW(&h("TaskbarCreated")) };

    let theme = ui::theme(unsafe { GetDpiForSystem() });
    let store = CallStore::new(shell::calls_dir()).ok();
    let calls = store.as_ref().map(CallStore::list).unwrap_or_default();
    let mut controller = Controller::new(settings.clone());
    if store.is_none() {
        controller.call_status_message = text::STATUS_CALL_STORAGE_UNAVAILABLE;
    }
    let mut platform = WinPlatform::new(store);

    // Never leave Pabulong Mic as the default input (fixed at launch, like
    // macOS; mid-session the menu offers the fix instead).
    devices::move_default_input_to_physical(settings.selected_input_device_uid.as_deref());
    controller.start_monitoring(&mut platform);
    shell::set_launch_at_login(settings.launch_at_login);

    let mut tray = Tray::new(tray_hwnd);
    tray.add(controller.enabled(), text::APP_NAME);

    APP.with(|cell| {
        *cell.borrow_mut() = Some(App {
            instance,
            controller,
            platform,
            settings_store,
            tray_hwnd,
            tray,
            theme,
            main: None,
            notices: None,
            devices: devices::list_inputs(),
            calls,
            virtual_mic_default: devices::virtual_mic_is_default_input(),
            last_slow: Instant::now(),
            last_calls: Instant::now(),
            taskbar_created,
        })
    });
    unsafe {
        SetTimer(Some(tray_hwnd), TICK_TIMER, TICK_MS, None);
    }

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let main = with_app(|a| a.main.as_ref().map(|m| m.hwnd)).flatten();
            if let Some(main) = main {
                if IsDialogMessageW(main, &msg).as_bool() {
                    continue;
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

impl App {
    fn persist(&mut self) {
        let _ = self.settings_store.save(&self.controller.settings);
    }

    fn settings_changed(&mut self) {
        self.controller.settings_changed(&mut self.platform);
        self.persist();
        self.refresh_ui();
    }

    fn tick(&mut self) {
        let now = Instant::now();
        self.controller.tick(&mut self.platform, now);
        if now.duration_since(self.last_slow) >= Duration::from_millis(500) {
            self.last_slow = now;
            // Surface (don't auto-change mid-session) a default-input problem.
            self.virtual_mic_default = devices::virtual_mic_is_default_input();
        }
        if self.main.is_some() && now.duration_since(self.last_calls) >= Duration::from_secs(2) {
            self.last_calls = now;
            self.calls = self.platform.store().map(CallStore::list).unwrap_or_default();
        }
        self.refresh_ui();
    }

    fn tooltip(&self) -> String {
        format!("{} — {}", text::APP_NAME, self.controller.menu_subtitle(&self.platform))
    }

    fn refresh_ui(&mut self) {
        let tip = self.tooltip();
        self.tray.update(self.controller.enabled() || self.controller.ring.producer_active, &tip, false);
        if self.main.is_some() {
            let vm = self.view_model();
            if let Some(main) = self.main.as_mut() {
                main.update(&vm);
            }
        }
    }

    fn menu_state(&self) -> MenuState {
        let c = &self.controller;
        let p = &self.platform;
        MenuState {
            subtitle: c.menu_subtitle(p).to_string(),
            processing_on: c.enabled(),
            show_recording: c.is_virtual_mic_in_use(p) || c.active_call.is_some(),
            recording_title: c.recording_title(p).to_string(),
            recording_subtitle: c.recording_subtitle(p, unix_now()),
            is_recording: c.active_call.is_some(),
            can_record: c.can_start_manual_recording(p),
            mic_access_denied: c.mic_access_denied,
            virtual_mic_is_default: self.virtual_mic_default,
        }
    }

    fn call_line(call: &CallSession) -> String {
        let (date, time) = shell::local_date_time(call.started_at);
        format!("{}    {} {} · {}", call.title, date, time, duration_text(call.duration_seconds(unix_now())))
    }

    fn view_model(&self) -> ViewModel {
        let c = &self.controller;
        let p = &self.platform;
        let s = &c.settings;
        let mut devices = vec![text::SYSTEM_DEFAULT.to_string()];
        devices.extend(self.devices.iter().map(|d| if d.is_default { format!("{}{}", d.name, text::DEFAULT_SUFFIX) } else { d.name.clone() }));
        let selected_device = s
            .selected_input_device_uid
            .as_ref()
            .and_then(|id| self.devices.iter().position(|d| &d.id == id))
            .map_or(0, |i| i + 1);
        let m = &c.metrics;
        let r = &c.ring;
        ViewModel {
            status: if c.enabled() { c.virtual_device_status().to_string() } else { text::OFF.to_string() },
            processing_on: c.enabled(),
            recording_title: c.recording_title(p).to_string(),
            recording_subtitle: if c.call_status_message.is_empty() {
                c.recording_subtitle(p, unix_now())
            } else {
                c.call_status_message.to_string()
            },
            is_recording: c.active_call.is_some(),
            can_record: c.can_start_manual_recording(p),
            calls: self.calls.iter().map(Self::call_line).collect(),
            devices,
            selected_device,
            suppression: s.suppression_mode,
            echo: s.echo_cancellation_enabled,
            voices: s.voice_isolation,
            bypass: s.bypass_enabled,
            auto_recording: s.automatic_call_recording_enabled,
            consent_needed: !s.recording_consent_acknowledged,
            local_recording: s.local_recording_enabled,
            system_audio: s.capture_system_audio_enabled,
            launch_at_login: s.launch_at_login,
            version: env!("CARGO_PKG_VERSION").to_string(),
            tiles: vec![
                (text::INPUT, format!("{:.3}", m.input_rms)),
                (text::OUTPUT, format!("{:.3}", m.output_rms)),
                (text::ENGINE, c.engine_name.to_string()),
                (text::MIC_CLIENTS, r.consumer_running_count.to_string()),
                (text::LATENCY, text::LATENCY_VALUE.to_string()),
                (text::VIRTUAL_MIC, c.virtual_device_status().to_string()),
                (text::ECHO_CANCELLATION, c.echo_status_text()),
                (text::ECHO_PATH, c.echo_path_text()),
                (text::BACKGROUND_VOICES, c.voice_gate_text()),
                (text::SPEAKER_REFERENCE, if c.echo_reference_active { text::TAPPING_SYSTEM_OUTPUT } else { text::NOT_RUNNING }.to_string()),
            ],
            health: format!(
                "Pabulong Mic driver v{} · {} underruns · {} overruns · {} frames skipped to keep latency low",
                r.driver_version, r.underrun_count, r.overrun_count, r.dropped_count
            ),
        }
    }

    fn open_main(&mut self) {
        if let Some(main) = &self.main {
            main.bring_to_front();
            return;
        }
        self.devices = devices::list_inputs();
        self.calls = self.platform.store().map(CallStore::list).unwrap_or_default();
        self.main = MainWindow::create(self.instance, &self.theme);
        self.refresh_ui();
    }

    fn export_diagnostics(&self) {
        let names: Vec<String> = self.devices.iter().map(|d| format!("{} [{}]", d.name, d.id)).collect();
        let json = diagnostics_json(&self.controller, env!("CARGO_PKG_VERSION"), &names);
        let path = std::env::temp_dir().join("PabulongDiagnostics.json");
        if std::fs::write(&path, json).is_ok() {
            shell::reveal(&path);
        }
    }

    fn start_manual_recording(&mut self) -> Action {
        if self.controller.start_manual_recording(&mut self.platform) {
            self.refresh_ui();
            Action::None
        } else {
            Action::Consent(AfterConsent::StartManualRecording)
        }
    }

    fn toggle_record(&mut self) -> Action {
        if self.controller.active_call.is_some() {
            self.controller.stop_active_call(&mut self.platform);
            self.refresh_ui();
            Action::None
        } else {
            self.start_manual_recording()
        }
    }

    fn after_consent(&mut self, accepted: bool, then: AfterConsent) -> Action {
        if !accepted {
            self.refresh_ui();
            return Action::None;
        }
        self.controller.settings.recording_consent_acknowledged = true;
        match then {
            AfterConsent::EnableAutoRecording => self.controller.settings.automatic_call_recording_enabled = true,
            AfterConsent::StartManualRecording | AfterConsent::Review => {}
        }
        self.settings_changed();
        match then {
            AfterConsent::StartManualRecording => self.start_manual_recording(),
            _ => Action::None,
        }
    }

    fn menu_command(&mut self, cmd: usize) -> Action {
        match cmd {
            tray::CMD_TOGGLE_PROCESSING => {
                let on = !self.controller.enabled();
                self.controller.set_processing_enabled(on, &mut self.platform);
                self.persist();
                self.refresh_ui();
                Action::None
            }
            tray::CMD_RECORD => self.toggle_record(),
            tray::CMD_MIC_SETTINGS => {
                shell::open("ms-settings:privacy-microphone");
                Action::None
            }
            tray::CMD_FIX_DEFAULT => {
                let preferred = self.controller.settings.selected_input_device_uid.clone();
                if !devices::move_default_input_to_physical(preferred.as_deref()) && devices::virtual_mic_is_default_input() {
                    // Could not switch programmatically: let the user do it.
                    shell::open("ms-settings:sound");
                }
                self.virtual_mic_default = devices::virtual_mic_is_default_input();
                Action::None
            }
            tray::CMD_OPEN => {
                self.open_main();
                Action::None
            }
            tray::CMD_QUIT => Action::Quit,
            _ => Action::None,
        }
    }

    fn checkbox_state(&self, id: i32) -> bool {
        let Some(main) = &self.main else { return false };
        unsafe { windows::Win32::UI::Controls::IsDlgButtonChecked(main.hwnd, id) == 1 }
    }

    fn combo_index(&self, id: i32) -> Option<usize> {
        let main = self.main.as_ref()?;
        let combo = unsafe { windows::Win32::UI::WindowsAndMessaging::GetDlgItem(Some(main.hwnd), id) }.ok()?;
        let i = unsafe { SendMessageW(combo, CB_GETCURSEL, None, None) }.0;
        (i >= 0).then_some(i as usize)
    }

    fn main_command(&mut self, id: i32, code: u32) -> Action {
        let s = &mut self.controller.settings;
        match id {
            window::NAV_CALLS | window::NAV_SETTINGS | window::NAV_DIAGNOSTICS => {
                let page = match id {
                    window::NAV_CALLS => Page::Calls,
                    window::NAV_SETTINGS => Page::Settings,
                    _ => Page::Diagnostics,
                };
                if page == Page::Calls {
                    self.calls = self.platform.store().map(CallStore::list).unwrap_or_default();
                }
                if let Some(main) = self.main.as_mut() {
                    main.set_page(page);
                }
                self.refresh_ui();
            }
            window::SIDEBAR_PROCESSING | window::S_PROCESSING => {
                let on = self.checkbox_state(id);
                self.controller.set_processing_enabled(on, &mut self.platform);
                self.persist();
                self.refresh_ui();
            }
            window::S_MIC if code == CBN_SELCHANGE => {
                let index = self.combo_index(id).unwrap_or(0);
                self.controller.settings.selected_input_device_uid = index.checked_sub(1).and_then(|i| self.devices.get(i)).map(|d| d.id.clone());
                self.settings_changed();
            }
            window::S_SUPPRESSION if code == CBN_SELCHANGE => {
                if let Some(mode) = self.combo_index(id).and_then(|i| SuppressionMode::ALL.get(i).copied()) {
                    self.controller.settings.suppression_mode = mode;
                    self.settings_changed();
                }
            }
            window::S_VOICES if code == CBN_SELCHANGE => {
                if let Some(level) = self.combo_index(id).and_then(|i| VoiceIsolation::ALL.get(i).copied()) {
                    self.controller.settings.voice_isolation = level;
                    self.settings_changed();
                }
            }
            window::S_ECHO => {
                s.echo_cancellation_enabled = !s.echo_cancellation_enabled;
                self.settings_changed();
            }
            window::S_BYPASS => {
                s.bypass_enabled = !s.bypass_enabled;
                self.settings_changed();
            }
            window::S_LOCALREC => {
                s.local_recording_enabled = !s.local_recording_enabled;
                self.settings_changed();
            }
            window::S_SYSAUDIO => {
                s.capture_system_audio_enabled = !s.capture_system_audio_enabled;
                self.settings_changed();
            }
            window::S_LOGIN => {
                s.launch_at_login = !s.launch_at_login;
                let on = s.launch_at_login;
                if !shell::set_launch_at_login(on) {
                    self.controller.pipeline_status = text::STATUS_LOGIN_ITEM_UNAVAILABLE;
                }
                self.settings_changed();
            }
            window::S_AUTOREC => {
                if s.automatic_call_recording_enabled {
                    s.automatic_call_recording_enabled = false;
                    self.settings_changed();
                } else if s.recording_consent_acknowledged {
                    s.automatic_call_recording_enabled = true;
                    self.settings_changed();
                } else {
                    // Leave it off until consent; the box re-syncs on refresh.
                    self.refresh_ui();
                    return Action::Consent(AfterConsent::EnableAutoRecording);
                }
            }
            window::S_REVIEW => return Action::Consent(AfterConsent::Review),
            window::S_OPEN_STORAGE => {
                if let Some(store) = self.platform.store() {
                    shell::open(&store.calls_dir().to_string_lossy());
                }
            }
            window::S_PRIVACY => shell::open(text::PRIVACY_URL),
            window::S_TERMS => shell::open(text::TERMS_URL),
            window::S_NOTICES => return Action::OpenNotices,
            window::REC_BUTTON => return self.toggle_record(),
            window::CALL_REVEAL => {
                if let Some(call) = self.main.as_ref().and_then(MainWindow::selected_call).and_then(|i| self.calls.get(i)) {
                    shell::reveal(&call.mixed_path());
                }
            }
            window::CALLS_LIST if code == LBN_DBLCLK => {
                if let Some(call) = self.main.as_ref().and_then(MainWindow::selected_call).and_then(|i| self.calls.get(i)) {
                    shell::open(&call.mixed_path().to_string_lossy());
                }
            }
            window::CALL_DELETE => {
                if let Some(i) = self.main.as_ref().and_then(MainWindow::selected_call) {
                    if let Some(call) = self.calls.get(i) {
                        return Action::ConfirmDelete(i, call.title.clone());
                    }
                }
            }
            window::D_REFRESH => {
                self.devices = devices::list_inputs();
                self.calls = self.platform.store().map(CallStore::list).unwrap_or_default();
                self.refresh_ui();
            }
            window::D_EXPORT => self.export_diagnostics(),
            _ => {}
        }
        Action::None
    }

    fn shutdown(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.tray_hwnd), TICK_TIMER);
        }
        self.controller.shutdown(&mut self.platform);
        self.persist();
        self.tray.remove();
        if let Some(main) = self.main.take() {
            main.close();
        }
    }
}

/// Runs the modal part of an action, then feeds the result back.
fn perform(action: Action) {
    match action {
        Action::None => {}
        Action::ShowMenu(state) => {
            let Some(hwnd) = with_app(|a| a.tray_hwnd) else { return };
            let cmd = tray::Tray::show_menu(hwnd, &state);
            if cmd != 0 {
                if let Some(next) = with_app(|a| a.menu_command(cmd)) {
                    perform(next);
                }
            }
        }
        Action::Consent(then) => {
            let parent = with_app(|a| a.main.as_ref().map(|m| m.hwnd)).flatten();
            let accepted = dialogs::recording_consent(parent);
            if let Some(next) = with_app(|a| a.after_consent(accepted, then)) {
                perform(next);
            }
        }
        Action::ConfirmDelete(index, title) => {
            let parent = with_app(|a| a.main.as_ref().map(|m| m.hwnd)).flatten();
            if dialogs::confirm_delete(parent, &title) {
                with_app(|a| {
                    if let (Some(store), Some(call)) = (a.platform.store(), a.calls.get(index)) {
                        let _ = store.delete(call);
                    }
                    a.calls = a.platform.store().map(CallStore::list).unwrap_or_default();
                    a.refresh_ui();
                });
            }
        }
        Action::OpenNotices => {
            with_app(|a| {
                if a.notices.is_none() {
                    a.notices = NoticesWindow::create(a.instance, &a.theme, a.main.as_ref().map(|m| m.hwnd));
                }
            });
        }
        Action::Quit => {
            with_app(App::shutdown);
            unsafe { PostQuitMessage(0) };
        }
    }
}

unsafe extern "system" fn tray_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wparam.0 == TICK_TIMER => {
            with_app(App::tick);
            LRESULT(0)
        }
        tray::WM_TRAY => {
            // NOTIFYICON_VERSION_4: the event is in the low word of lParam.
            let event = (lparam.0 & 0xFFFF) as u32;
            if event == WM_LBUTTONUP || event == WM_RBUTTONUP || event == windows::Win32::UI::WindowsAndMessaging::WM_CONTEXTMENU || event == windows::Win32::UI::Shell::NIN_SELECT {
                if let Some(state) = with_app(|a| a.menu_state()) {
                    perform(Action::ShowMenu(state));
                }
            }
            LRESULT(0)
        }
        WM_SHOW_MAIN => {
            with_app(App::open_main);
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            // Taskbar light/dark switch: redraw the tray icon.
            with_app(|a| {
                let tip = a.tooltip();
                let on = a.controller.enabled() || a.controller.ring.producer_active;
                a.tray.update(on, &tip, true);
            });
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_QUERYENDSESSION => LRESULT(1),
        WM_ENDSESSION => {
            // Log-off, shutdown, or the installer's Restart Manager closing us.
            if wparam.0 != 0 {
                with_app(App::shutdown);
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => {
            if let Some(true) = with_app(|a| msg == a.taskbar_created && msg != 0) {
                // Explorer restarted: put the icon back.
                with_app(|a| {
                    let tip = a.tooltip();
                    let on = a.controller.enabled();
                    a.tray.add(on, &tip);
                });
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = window::command_id(wparam);
            let code = window::command_code(wparam);
            if let Some(action) = with_app(|a| a.main_command(id, code)) {
                perform(action);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            with_app(|a| {
                if let Some(m) = &a.main {
                    if m.hwnd == hwnd {
                        m.layout();
                    }
                }
            });
            LRESULT(0)
        }
        WM_PAINT => {
            if window::paint(hwnd) {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let dc = HDC(wparam.0 as *mut _);
            let control = HWND(lparam.0 as *mut _);
            window::control_color(dc, control).unwrap_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam))
        }
        WM_CLOSE => {
            // Closing the window keeps Pabulong running in the tray.
            with_app(|a| {
                if let Some(m) = a.main.take() {
                    m.close();
                }
            });
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => window::default_proc(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn notices_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            with_app(|a| {
                if let Some(n) = a.notices.as_ref().filter(|n| n.is(hwnd)) {
                    n.layout();
                }
            });
            LRESULT(0)
        }
        WM_COMMAND if window::command_id(wparam) == dialogs::IDC_NOTICES_DONE => {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            LRESULT(0)
        }
        WM_CLOSE => {
            with_app(|a| a.notices = None);
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
