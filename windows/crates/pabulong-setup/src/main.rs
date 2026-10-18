//! Installs or removes the root-enumerated "Pabulong Mic" device.
//!
//! A virtual audio device has no hardware to plug in, so Windows needs a
//! device node created for it (what `devcon install` does). The MSI runs:
//!
//!     pabulong-setup install  <path\to\PabulongMic.inf>
//!     pabulong-setup uninstall <path\to\PabulongMic.inf>
//!     pabulong-setup status
//!
//! Exit codes: 0 done, 1 failed. When Windows needs a restart to finish,
//! it says so on stderr and still exits 0 (pass `--reboot-code` to get 3010
//! instead). Needs administrator rights (the MSI runs it elevated).

#[cfg(windows)]
mod imp {
    use windows::core::{BOOL, HSTRING, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        DiUninstallDevice, DiUninstallDriverW, SetupDiCallClassInstaller, SetupDiCreateDeviceInfoList, SetupDiCreateDeviceInfoW,
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW,
        SetupDiSetDeviceRegistryPropertyW, UpdateDriverForPlugAndPlayDevicesW, DICD_GENERATE_ID, DIF_REGISTERDEVICE,
        DIUNINSTALLDRIVER_FLAGS, GUID_DEVCLASS_MEDIA, HDEVINFO, INSTALLFLAG_FORCE, SETUP_DI_GET_CLASS_DEVS_FLAGS, SPDRP_HARDWAREID,
        SP_DEVINFO_DATA,
    };
    use windows::Win32::Foundation::HWND;

    /// Must match the hardware id in PabulongMic.inx.
    pub const HARDWARE_ID: &str = r"Root\PabulongMic";
    const CLASS_NAME: &str = "MEDIA";

    pub enum Outcome {
        Done,
        RebootNeeded,
    }

    fn multi_sz(s: &str) -> Vec<u8> {
        let mut units: Vec<u16> = s.encode_utf16().collect();
        units.extend([0, 0]);
        units.iter().flat_map(|u| u.to_le_bytes()).collect()
    }

    struct DevInfo(HDEVINFO);
    impl Drop for DevInfo {
        fn drop(&mut self) {
            unsafe {
                let _ = SetupDiDestroyDeviceInfoList(self.0);
            }
        }
    }

    fn devinfo_data() -> SP_DEVINFO_DATA {
        SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() }
    }

    /// Calls `f` for every MEDIA-class device (present or not) whose hardware
    /// ids include ours.
    fn for_each_ours(mut f: impl FnMut(&DevInfo, &SP_DEVINFO_DATA)) -> windows::core::Result<usize> {
        let set = DevInfo(unsafe { SetupDiGetClassDevsW(Some(&GUID_DEVCLASS_MEDIA), PCWSTR::null(), None, SETUP_DI_GET_CLASS_DEVS_FLAGS(0))? });
        let mut found = 0;
        for index in 0.. {
            let mut data = devinfo_data();
            if unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut data) }.is_err() {
                break;
            }
            let mut buf = vec![0u8; 2048];
            if unsafe { SetupDiGetDeviceRegistryPropertyW(set.0, &data, SPDRP_HARDWAREID, None, Some(&mut buf), None) }.is_err() {
                continue;
            }
            let units: Vec<u16> = buf.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
            let ids = String::from_utf16_lossy(&units);
            if ids.split('\0').any(|id| id.eq_ignore_ascii_case(HARDWARE_ID)) {
                found += 1;
                f(&set, &data);
            }
        }
        Ok(found)
    }

    pub fn status() -> windows::core::Result<usize> {
        for_each_ours(|_, _| {})
    }

    pub fn install(inf: &str) -> windows::core::Result<Outcome> {
        if status()? == 0 {
            let set = DevInfo(unsafe { SetupDiCreateDeviceInfoList(Some(&GUID_DEVCLASS_MEDIA), None)? });
            let mut data = devinfo_data();
            unsafe {
                SetupDiCreateDeviceInfoW(set.0, &HSTRING::from(CLASS_NAME), &GUID_DEVCLASS_MEDIA, PCWSTR::null(), None, DICD_GENERATE_ID, Some(&mut data))?;
                SetupDiSetDeviceRegistryPropertyW(set.0, &mut data, SPDRP_HARDWAREID, Some(&multi_sz(HARDWARE_ID)))?;
                SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set.0, Some(&data))?;
            }
        }
        let mut reboot = BOOL(0);
        unsafe {
            UpdateDriverForPlugAndPlayDevicesW(None, &HSTRING::from(HARDWARE_ID), &HSTRING::from(inf), INSTALLFLAG_FORCE, Some(&mut reboot))?;
        }
        Ok(if reboot.as_bool() { Outcome::RebootNeeded } else { Outcome::Done })
    }

    pub fn uninstall(inf: Option<&str>) -> windows::core::Result<Outcome> {
        let mut reboot = false;
        for_each_ours(|set, data| {
            let mut needs = BOOL(0);
            if unsafe { DiUninstallDevice(HWND::default(), set.0, data, 0, Some(&mut needs)) }.is_ok() {
                reboot |= needs.as_bool();
            }
        })?;
        if let Some(inf) = inf {
            // Removes the package from the driver store (Windows 10 1703+).
            let mut needs = BOOL(0);
            if unsafe { DiUninstallDriverW(None, &HSTRING::from(inf), DIUNINSTALLDRIVER_FLAGS(0), Some(&mut needs)) }.is_ok() {
                reboot |= needs.as_bool();
            }
        }
        Ok(if reboot { Outcome::RebootNeeded } else { Outcome::Done })
    }
}

#[cfg(windows)]
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let reboot_code = args.iter().any(|a| a == "--reboot-code");
    args.retain(|a| a != "--reboot-code");
    let inf = args.get(1).map(|p| std::fs::canonicalize(p).map(|p| p.display().to_string().trim_start_matches(r"\\?\").to_string()).unwrap_or_else(|_| p.clone()));
    let result = match args.first().map(String::as_str) {
        Some("install") => match inf.as_deref() {
            Some(inf) => imp::install(inf),
            None => {
                eprintln!("usage: pabulong-setup install <PabulongMic.inf>");
                std::process::exit(1);
            }
        },
        Some("uninstall") => imp::uninstall(inf.as_deref()),
        Some("status") => {
            match imp::status() {
                Ok(n) => {
                    println!("{} Pabulong Mic device(s) installed", n);
                    std::process::exit(if n > 0 { 0 } else { 2 });
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("usage: pabulong-setup install|uninstall <PabulongMic.inf> | status");
            std::process::exit(1);
        }
    };
    match result {
        Ok(imp::Outcome::Done) => {}
        Ok(imp::Outcome::RebootNeeded) => {
            eprintln!("pabulong-setup: restart Windows to finish");
            if reboot_code {
                std::process::exit(3010);
            }
        }
        Err(e) => {
            eprintln!("pabulong-setup: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("pabulong-setup only runs on Windows");
    std::process::exit(1);
}
