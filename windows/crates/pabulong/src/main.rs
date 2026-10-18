//! Pabulong for Windows: a tray app that cleans up your microphone and feeds
//! it to the "Pabulong Mic" virtual microphone, using the same Rust core as
//! the macOS app. It makes no network connections.
//!
//! Everything outside `win/` is platform-independent and unit-tested on any
//! OS; `win/` is the Win32 / WASAPI / driver glue.

#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
// Off Windows only the portable modules build (for tests), so much is unused.
#![cfg_attr(not(windows), allow(dead_code))]

mod chain;
mod controller;
mod convert;
mod framer;
mod json;
mod mic_path;
mod recording;
mod settings;
mod text;
#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    win::app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Pabulong for Windows builds its app on Windows only; run `cargo test` here for the shared logic.");
    std::process::exit(1);
}
