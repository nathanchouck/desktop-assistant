//! Launching Start menu apps the way Start does: execute the AppsFolder item.

use std::mem::size_of;
use std::ptr::null_mut;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ILFree, SHParseDisplayName, ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_IDLIST,
    SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY, SW_SHOWNORMAL};

use super::apps_folder_path;

/// Must run on an STA thread. Declining the UAC prompt counts as success.
pub fn launch_app(target: &str, as_admin: bool) -> windows::core::Result<()> {
    // SAFETY: the PIDL from SHParseDisplayName is freed after use; the
    // SHELLEXECUTEINFOW only borrows it for the duration of the call.
    unsafe {
        let mut pidl: *mut ITEMIDLIST = null_mut();
        SHParseDisplayName(
            &apps_folder_path(target),
            None::<&IBindCtx>,
            &mut pidl,
            0,
            None,
        )?;

        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            // NOASYNC: finish before this short-lived thread exits.
            // FLAG_NO_UI: report errors to us instead of a Windows dialog.
            fMask: SEE_MASK_IDLIST | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            lpVerb: if as_admin {
                w!("runas")
            } else {
                PCWSTR::null()
            },
            lpIDList: pidl.cast(),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // Let the new app come to the front over the launcher.
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        let result = ShellExecuteExW(&mut info);
        ILFree(Some(pidl));

        match result {
            Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => Ok(()),
            other => other,
        }
    }
}
