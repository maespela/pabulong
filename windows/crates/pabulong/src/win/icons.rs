//! Icons rendered from the logo kit by `windows/assets/make-icons.py`,
//! embedded in the binary.

use windows::Win32::UI::WindowsAndMessaging::{CreateIconFromResourceEx, HICON, LR_DEFAULTCOLOR};

pub const APP: &[u8] = include_bytes!("../../../../assets/pabulong.ico");
pub const TRAY_LIGHT_ON: &[u8] = include_bytes!("../../../../assets/tray-light-on.ico");
pub const TRAY_LIGHT_OFF: &[u8] = include_bytes!("../../../../assets/tray-light-off.ico");
pub const TRAY_DARK_ON: &[u8] = include_bytes!("../../../../assets/tray-dark-on.ico");
pub const TRAY_DARK_OFF: &[u8] = include_bytes!("../../../../assets/tray-dark-off.ico");

/// Picks the image in an .ico file closest to `size` px (preferring larger)
/// and creates an HICON from it.
pub fn load(ico: &[u8], size: i32) -> Option<HICON> {
    let count = u16::from_le_bytes([*ico.get(4)?, *ico.get(5)?]) as usize;
    let mut best: Option<(i32, usize, usize)> = None; // (score, offset, len)
    for i in 0..count {
        let e = ico.get(6 + i * 16..6 + i * 16 + 16)?;
        let dim = if e[0] == 0 { 256 } else { i32::from(e[0]) };
        let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let offset = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        // Exact or next larger size wins; smaller sizes are a last resort.
        let score = if dim >= size { dim - size } else { 1000 + size - dim };
        if best.is_none_or(|(s, _, _)| score < s) {
            best = Some((score, offset, len));
        }
    }
    let (_, offset, len) = best?;
    let bits = ico.get(offset..offset + len)?;
    unsafe { CreateIconFromResourceEx(bits, true, 0x0003_0000, size, size, LR_DEFAULTCOLOR).ok() }
}

/// True when the taskbar uses the light theme (Windows 10 1903+).
pub fn taskbar_is_light() -> bool {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut value = 0u32;
    let mut size = 4u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &super::h(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            &super::h("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    ok.is_ok() && value != 0
}

pub fn tray(on: bool, size: i32) -> Option<HICON> {
    let ico = match (taskbar_is_light(), on) {
        (true, true) => TRAY_LIGHT_ON,
        (true, false) => TRAY_LIGHT_OFF,
        (false, true) => TRAY_DARK_ON,
        (false, false) => TRAY_DARK_OFF,
    };
    load(ico, size)
}
