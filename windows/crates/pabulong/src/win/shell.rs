//! Folders, links, Explorer, login item and local-time formatting.

use super::{h, wide};
use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_SUCCESS, SYSTEMTIME};
use windows::Win32::Globalization::{GetDateFormatEx, GetTimeFormatEx, DATE_SHORTDATE, TIME_NOSECONDS};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::{
    ILCreateFromPathW, ILFree, SHGetKnownFolderPath, SHOpenFolderAndSelectItems, ShellExecuteW, FOLDERID_LocalAppData, FOLDERID_RoamingAppData,
    KF_FLAG_CREATE,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_CREATE, None).ok()?;
        let path = p.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(p.0 as *const _));
        path
    }
}

/// `%APPDATA%\Pabulong` (settings).
pub fn settings_dir() -> PathBuf {
    known_folder(&FOLDERID_RoamingAppData).unwrap_or_else(std::env::temp_dir).join("Pabulong")
}

/// `%LOCALAPPDATA%\Pabulong\Calls` (recordings stay on this PC, never roam).
pub fn calls_dir() -> PathBuf {
    known_folder(&FOLDERID_LocalAppData).unwrap_or_else(std::env::temp_dir).join("Pabulong").join("Calls")
}

/// Opens a URL or folder with the user's default handler. This hands the
/// link to the browser; Pabulong itself makes no network connection.
pub fn open(target: &str) {
    unsafe {
        ShellExecuteW(None, &h("open"), &h(target), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

/// Opens Explorer with `path` selected.
pub fn reveal(path: &Path) {
    unsafe {
        let pidl = ILCreateFromPathW(&h(&path.to_string_lossy()));
        if pidl.is_null() {
            if let Some(parent) = path.parent() {
                open(&parent.to_string_lossy());
            }
            return;
        }
        let _ = SHOpenFolderAndSelectItems(pidl, None, 0);
        ILFree(Some(pidl));
    }
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// "Launch at Login": the per-user Run key (no admin rights needed).
pub fn set_launch_at_login(enabled: bool) -> bool {
    unsafe {
        let mut key = HKEY::default();
        if RegCreateKeyExW(HKEY_CURRENT_USER, &h(RUN_KEY), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut key, None)
            != ERROR_SUCCESS
        {
            return false;
        }
        let ok = if enabled {
            let exe = std::env::current_exe().map(|p| format!("\"{}\"", p.display())).unwrap_or_default();
            let data = wide(&exe);
            let bytes = std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2);
            RegSetValueExW(key, &h(crate::text::APP_NAME), None, REG_SZ, Some(bytes)) == ERROR_SUCCESS
        } else {
            let r = RegDeleteValueW(key, &h(crate::text::APP_NAME));
            r == ERROR_SUCCESS || r.0 == 2 // ERROR_FILE_NOT_FOUND: already off
        };
        let _ = RegCloseKey(key);
        ok
    }
}

fn format_with(f: impl Fn(Option<&mut [u16]>) -> i32) -> String {
    let n = f(None);
    if n <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize];
    let n = f(Some(&mut buf));
    String::from_utf16_lossy(&buf[..(n.max(1) as usize - 1)])
}

/// Local time in the user's locale, for call titles ("Call 10/17/2026 3:04 PM").
pub fn local_timestamp() -> String {
    let now: SYSTEMTIME = unsafe { GetLocalTime() };
    let date = format_with(|buf| unsafe { GetDateFormatEx(PCWSTR::null(), DATE_SHORTDATE, Some(&now), PCWSTR::null(), buf, PCWSTR::null()) });
    let time = format_with(|buf| unsafe { GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(&now), PCWSTR::null(), buf) });
    format!("{date} {time}")
}

/// Unix seconds → local date and time strings for the Calls list.
pub fn local_date_time(unix: f64) -> (String, String) {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let ticks = ((unix + 11_644_473_600.0) * 10_000_000.0) as u64;
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            return (String::new(), String::new());
        }
    }
    let date = format_with(|buf| unsafe { GetDateFormatEx(PCWSTR::null(), DATE_SHORTDATE, Some(&local), PCWSTR::null(), buf, PCWSTR::null()) });
    let time = format_with(|buf| unsafe { GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(&local), PCWSTR::null(), buf) });
    (date, time)
}
