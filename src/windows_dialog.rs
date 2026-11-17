//! Native dialogs. These functions are never loaded by simulation tests.
use std::ptr;
use windows_sys::Win32::UI::{Controls::*, WindowsAndMessaging::*};

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn show(heading: &str, content: &str, confirmation: bool) -> Option<i32> {
    let title = wide("Boothop");
    let heading = wide(heading);
    let content = wide(content);
    let restart = wide("Restart");
    let cancel = wide("Cancel");
    let buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: IDYES,
            pszButtonText: restart.as_ptr(),
        },
        TASKDIALOG_BUTTON {
            nButtonID: IDCANCEL,
            pszButtonText: cancel.as_ptr(),
        },
    ];
    let mut config = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        pszWindowTitle: title.as_ptr(),
        pszMainInstruction: heading.as_ptr(),
        pszContent: if content.len() > 1 {
            content.as_ptr()
        } else {
            ptr::null()
        },
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
        nDefaultButton: if confirmation { IDCANCEL } else { IDOK },
        ..Default::default()
    };
    if confirmation {
        config.cButtons = buttons.len() as u32;
        config.pButtons = buttons.as_ptr();
    } else {
        config.dwCommonButtons = TDCBF_OK_BUTTON;
        config.Anonymous1.pszMainIcon = TD_ERROR_ICON;
    }
    let mut answer = 0;
    // All strings and buttons outlive this synchronous call. No callback is installed.
    let result =
        unsafe { TaskDialogIndirect(&config, &mut answer, ptr::null_mut(), ptr::null_mut()) };
    (result >= 0).then_some(answer)
}

pub fn confirmation(message: &str) -> Option<bool> {
    let (heading, content) = message.split_once('\n').unwrap_or((message, ""));
    show(heading, content, true).map(|answer| answer == IDYES)
}

pub fn error(message: &str) {
    if show("Unable to restart", message, false).is_none() {
        // Keep launch failures visible even if common controls cannot create a dialog.
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                wide(message).as_ptr(),
                wide("Boothop").as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
            );
        }
    }
}
