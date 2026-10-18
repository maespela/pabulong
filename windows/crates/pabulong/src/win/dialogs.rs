//! Task dialogs with the macOS wording, and the Third-Party Notices window.

use super::{icons, shell, ui, wide};
use crate::text;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::{
    TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOG_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION,
    TDF_POSITION_RELATIVE_TO_WINDOW, TDF_USE_HICON_MAIN,
};

const ID_FIRST: i32 = 100;

/// Shows a task dialog with custom buttons; returns the index of the button
/// pressed, or None if it was dismissed (Esc / close box) or failed.
fn ask(parent: Option<HWND>, instruction: &str, content: &str, buttons: &[&str], default: usize, cancellable: bool) -> Option<usize> {
    let title = wide(text::APP_NAME);
    let instruction = wide(instruction);
    let content = wide(content);
    let labels: Vec<Vec<u16>> = buttons.iter().map(|b| wide(b)).collect();
    let items: Vec<TASKDIALOG_BUTTON> = labels
        .iter()
        .enumerate()
        .map(|(i, l)| TASKDIALOG_BUTTON { nButtonID: ID_FIRST + i as i32, pszButtonText: PCWSTR(l.as_ptr()) })
        .collect();
    let icon = icons::load(icons::APP, 32);
    let mut flags = TDF_POSITION_RELATIVE_TO_WINDOW;
    if icon.is_some() {
        flags |= TDF_USE_HICON_MAIN;
    }
    if cancellable {
        flags |= TDF_ALLOW_DIALOG_CANCELLATION;
    }
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: parent.unwrap_or_default(),
        dwFlags: flags,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        Anonymous1: TASKDIALOGCONFIG_0 { hMainIcon: icon.unwrap_or_default() },
        pszMainInstruction: PCWSTR(instruction.as_ptr()),
        pszContent: PCWSTR(content.as_ptr()),
        cButtons: items.len() as u32,
        pButtons: items.as_ptr(),
        nDefaultButton: ID_FIRST + default as i32,
        ..Default::default()
    };
    let mut pressed = 0i32;
    let ok = unsafe { TaskDialogIndirect(&config, Some(&mut pressed), None, None) }.is_ok();
    if let Some(icon) = icon {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(icon);
        }
    }
    let index = pressed - ID_FIRST;
    (ok && index >= 0 && (index as usize) < buttons.len()).then_some(index as usize)
}

/// First-run click-wrap (`presentTermsAgreementIfNeeded`). Returns true when
/// the user agreed; false means quit.
pub fn terms_agreement() -> bool {
    loop {
        match ask(None, text::WELCOME_TITLE, text::WELCOME_BODY, &[text::I_AGREE, text::READ_TERMS, text::QUIT], 0, false) {
            Some(0) => return true,
            Some(1) => {
                shell::open(text::TERMS_URL);
                shell::open(text::PRIVACY_URL);
            }
            _ => return false,
        }
    }
}

/// Asked once, before the first recording. True = "I Understand".
pub fn recording_consent(parent: Option<HWND>) -> bool {
    ask(parent, text::CONSENT_TITLE, text::CONSENT_BODY, &[text::I_UNDERSTAND, text::CANCEL], 0, true) == Some(0)
}

pub fn confirm_delete(parent: Option<HWND>, title: &str) -> bool {
    ask(parent, text::DELETE_CALL_TITLE, title, &[text::DELETE, text::CANCEL], 1, true) == Some(0)
}

/// Bundled third-party license notices (RNNoise's BSD license requires them).
pub const NOTICES: &str = include_str!("../../../../../THIRD_PARTY_NOTICES.md");

/// Fills `parent` (a plain window) with the notices and a Done button.
pub fn notices_contents(parent: HWND, theme: &ui::Theme) -> (HWND, HWND, HWND) {
    let heading = ui::label(parent, text::THIRD_PARTY_NOTICES, theme.heading);
    // EDIT controls want CRLF line ends.
    let body = ui::readonly_text(parent, &NOTICES.replace("\r\n", "\n").replace('\n', "\r\n"), theme.caption);
    let done = ui::button(parent, text::DONE, IDC_NOTICES_DONE, theme.body);
    (heading, body, done)
}

pub const IDC_NOTICES_DONE: i32 = 900;
