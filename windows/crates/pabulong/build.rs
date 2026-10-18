//! Windows/MSVC only: embeds the application manifest (Common Controls v6
//! for task dialogs, per-monitor DPI awareness) and, when the Windows SDK's
//! `rc.exe` is on PATH (a Visual Studio developer shell, as in CI), the icon
//! and version resources. Does nothing on other targets.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Pabulong.exe.manifest");
    println!("cargo:rerun-if-changed=../../assets/pabulong.ico");
    let windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !(windows && msvc) {
        return;
    }
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let manifest = dir.join("Pabulong.exe.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let icon = dir.join("../../assets/pabulong.ico").canonicalize().unwrap_or_default();
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts: Vec<u32> = version.split(['.', '-']).filter_map(|p| p.parse().ok()).collect();
    parts.resize(4, 0);
    let numeric = format!("{},{},{},{}", parts[0], parts[1], parts[2], parts[3]);
    let rc = format!(
        r#"#include <winver.h>
1 ICON "{icon}"
VS_VERSION_INFO VERSIONINFO
 FILEVERSION {numeric}
 PRODUCTVERSION {numeric}
 FILEOS VOS_NT_WINDOWS32
 FILETYPE VFT_APP
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Next Matter Pro"
      VALUE "FileDescription", "Pabulong"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "Pabulong"
      VALUE "LegalCopyright", "\xA9 2026 Next Matter Pro. MIT License."
      VALUE "OriginalFilename", "Pabulong.exe"
      VALUE "ProductName", "Pabulong"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = icon.display().to_string().replace('\\', "\\\\"),
    );
    let rc_path = out.join("pabulong.rc");
    let res_path = out.join("pabulong.res");
    std::fs::write(&rc_path, rc).unwrap();
    match Command::new("rc.exe").args(["/nologo", "/fo"]).arg(&res_path).arg(&rc_path).status() {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg-bins={}", res_path.display()),
        _ => println!("cargo:warning=rc.exe not found or failed: Pabulong.exe will have no icon/version resource (run from a VS developer shell)"),
    }
}
