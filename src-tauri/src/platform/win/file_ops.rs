//! File actions from the action menu: "Open with" and "Delete to Recycle
//! Bin", done the way Explorer does them.

use windows::core::{HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::System::Com::{CoCreateInstance, IBindCtx, CLSCTX_ALL};
use windows::Win32::UI::Shell::{
    FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName, SHOpenWithDialog,
    COPYENGINE_E_USER_CANCELLED, FOFX_ADDUNDORECORD, FOFX_RECYCLEONDELETE, FOF_ALLOWUNDO,
    FOF_NOCONFIRMATION, FOF_WANTNUKEWARNING, OAIF_ALLOW_REGISTRATION, OAIF_EXEC,
    OAIF_REGISTER_EXT, OPENASINFO,
};
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};

/// Shows the "How do you want to open this file?" dialog; the program you
/// pick opens the file, and "Always use this app" works as in Explorer.
/// No owner window: the launcher hides as soon as the dialog takes focus,
/// so it can't hold the dialog. Blocks until the dialog closes, so run it
/// on its own STA thread.
pub fn open_with(path: &str) -> windows::core::Result<()> {
    let file = HSTRING::from(path);
    let info = OPENASINFO {
        pcszFile: PCWSTR(file.as_ptr()),
        pcszClass: PCWSTR::null(),
        oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_REGISTER_EXT | OAIF_EXEC,
    };
    // SAFETY: `info` and the string it points to outlive the call.
    let result = unsafe {
        // The dialog (and the program picked) may come to the front.
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        SHOpenWithDialog(None, &info)
    };
    match result {
        Err(e) if is_cancel(e.code()) => Ok(()),
        other => other,
    }
}

/// Moves `path` to the Recycle Bin with the shell's file operation, so
/// Explorer's Undo (Ctrl+Z) can bring it back. We already asked in the
/// footer, so Windows doesn't ask again, except when the item can't be
/// recycled (too big, a network or removable drive): then it asks before
/// deleting it for good. Returns false when it stayed.
pub fn trash(path: &str) -> windows::core::Result<bool> {
    // SAFETY: COM calls on an STA thread (the caller's contract); every
    // interface is used only while alive.
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path), None::<&IBindCtx>)?;
        let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
        op.SetOperationFlags(
            FOF_ALLOWUNDO
                | FOFX_RECYCLEONDELETE
                | FOFX_ADDUNDORECORD
                | FOF_NOCONFIRMATION
                | FOF_WANTNUKEWARNING,
        )?;
        op.DeleteItem(&item, None)?;
        match op.PerformOperations() {
            Err(e) if is_cancel(e.code()) => return Ok(false),
            other => other?,
        }
        Ok(!op.GetAnyOperationsAborted()?.as_bool())
    }
}

fn is_cancel(code: HRESULT) -> bool {
    code == ERROR_CANCELLED.to_hresult() || code == COPYENGINE_E_USER_CANCELLED
}
