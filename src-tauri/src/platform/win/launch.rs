//! Opening things the way Explorer and Start do: execute the shell item.

use std::mem::size_of;
use std::ptr::null_mut;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ILFree, SHOpenFolderAndSelectItems, SHParseDisplayName, ShellExecuteExW, SEE_MASK_FLAG_NO_UI,
    SEE_MASK_IDLIST, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY, SW_SHOWNORMAL};

use super::apps_folder_path;

/// Opens a Start menu app. Must run on an STA thread. Declining the UAC
/// prompt counts as success.
pub fn launch_app(target: &str, as_admin: bool) -> windows::core::Result<()> {
    execute(&apps_folder_path(target), as_admin)
}

/// Opens a file or folder with its default program. Must run on an STA thread.
pub fn open_path(path: &str) -> windows::core::Result<()> {
    execute(&HSTRING::from(path), false)
}

/// Opens the containing folder with the item selected. Must run on an STA thread.
pub fn reveal(path: &str) -> windows::core::Result<()> {
    let pidl = Pidl::parse(&HSTRING::from(path))?;
    // SAFETY: `pidl` is valid for the call; no child items are passed.
    unsafe { SHOpenFolderAndSelectItems(pidl.0, None, 0) }
}

fn execute(parsing_name: &HSTRING, as_admin: bool) -> windows::core::Result<()> {
    let pidl = Pidl::parse(parsing_name)?;
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
        lpIDList: pidl.0.cast(),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` and the PIDL it borrows are valid for the call.
    let result = unsafe {
        // Let the opened window come to the front over the launcher.
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        ShellExecuteExW(&mut info)
    };
    match result {
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => Ok(()),
        other => other,
    }
}

/// An item ID list from `SHParseDisplayName`, freed on drop.
struct Pidl(*mut ITEMIDLIST);

impl Pidl {
    fn parse(name: &HSTRING) -> windows::core::Result<Self> {
        let mut pidl: *mut ITEMIDLIST = null_mut();
        // SAFETY: `pidl` receives a newly allocated list that `Drop` frees.
        unsafe { SHParseDisplayName(name, None::<&IBindCtx>, &mut pidl, 0, None)? };
        Ok(Self(pidl))
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        // SAFETY: allocated by SHParseDisplayName and not freed elsewhere.
        unsafe { ILFree(Some(self.0.cast_const())) };
    }
}
