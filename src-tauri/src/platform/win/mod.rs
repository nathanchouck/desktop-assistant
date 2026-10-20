//! Windows shell integration via COM (the `windows` crate).

mod apps;
mod icon;
mod launch;
pub mod sta;

pub use apps::list_apps;
pub use icon::app_icon;
pub use launch::launch_app;

use windows::core::HSTRING;

/// The shell path for a Start menu app, e.g. `shell:AppsFolder\<AUMID>`.
fn apps_folder_path(target: &str) -> HSTRING {
    HSTRING::from(format!("shell:AppsFolder\\{target}"))
}
