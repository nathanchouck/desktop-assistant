//! Lists the Start menu's "All apps" (the AppsFolder): desktop programs and
//! Store apps alike, each with a display name and a launchable parsing name.

use windows::Win32::System::Com::{CoTaskMemFree, IBindCtx};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, SHGetKnownFolderItem,
    KF_FLAG_DEFAULT, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};

use crate::platform::RawApp;

/// Must run on an STA thread.
pub fn list_apps() -> windows::core::Result<Vec<RawApp>> {
    // SAFETY: COM calls on an STA thread; every string the shell returns is
    // freed in `display_name`.
    unsafe {
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)?;
        let items: IEnumShellItems = folder.BindToHandler(None::<&IBindCtx>, &BHID_EnumItems)?;

        let mut apps = Vec::new();
        loop {
            let mut slot: [Option<IShellItem>; 1] = [None];
            let mut fetched = 0u32;
            items.Next(&mut slot, Some(&mut fetched))?;
            let Some(item) = slot[0].take() else { break };
            if fetched == 0 {
                break;
            }
            let name = display_name(&item, SIGDN_NORMALDISPLAY);
            let target = display_name(&item, SIGDN_PARENTRELATIVEPARSING);
            if let (Ok(name), Ok(target)) = (name, target) {
                apps.push(RawApp { target, name });
            }
        }
        Ok(apps)
    }
}

unsafe fn display_name(item: &IShellItem, kind: SIGDN) -> windows::core::Result<String> {
    // SAFETY: GetDisplayName returns a CoTaskMem string we own and free here.
    unsafe {
        let raw = item.GetDisplayName(kind)?;
        let text = String::from_utf16_lossy(raw.as_wide());
        CoTaskMemFree(Some(raw.0 as *const _));
        Ok(text)
    }
}
