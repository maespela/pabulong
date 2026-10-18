//! Win32, WASAPI and driver glue. Everything here is `cfg(windows)`.

pub mod app;
pub mod devices;
pub mod dialogs;
pub mod driver;
pub mod engine;
pub mod icons;
pub mod shell;
pub mod tray;
pub mod ui;
pub mod wasapi;
pub mod window;

use windows::core::HSTRING;

/// NUL-terminated UTF-16 for APIs that keep the pointer only during the call.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn h(s: &str) -> HSTRING {
    HSTRING::from(s)
}

/// Copies `s` into a fixed UTF-16 buffer (truncating, always NUL-terminated).
pub fn copy_to_fixed(s: &str, out: &mut [u16]) {
    let mut n = 0;
    for unit in s.encode_utf16() {
        if n + 1 >= out.len() {
            break;
        }
        out[n] = unit;
        n += 1;
    }
    out[n] = 0;
}

/// Nanoseconds on the QPC clock, the host clock the core aligns mic and
/// speaker audio on (the macOS app uses mach host time).
pub fn qpc_now_ns() -> u64 {
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    let mut ticks = 0i64;
    let mut freq = 0i64;
    unsafe {
        let _ = QueryPerformanceCounter(&mut ticks);
        let _ = QueryPerformanceFrequency(&mut freq);
    }
    if freq <= 0 {
        return 0;
    }
    ((ticks as i128 * 1_000_000_000) / freq as i128) as u64
}
