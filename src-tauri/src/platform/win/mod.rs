//! Windows shell integration via COM (the `windows` crate), plus Everything.

mod apps;
pub mod everything;
mod icon;
mod launch;
pub mod sta;

pub use apps::list_apps;
pub use launch::{launch_app, open_path, reveal};

use windows::core::HSTRING;

use crate::platform::{IconSource, Image};

/// The shell path for a Start menu app, e.g. `shell:AppsFolder\<AUMID>`.
fn apps_folder_path(target: &str) -> HSTRING {
    HSTRING::from(format!("shell:AppsFolder\\{target}"))
}

/// Must run on an STA thread.
pub fn icon(source: &IconSource, size: u32) -> windows::core::Result<Image> {
    let parsing_name = match source {
        IconSource::App(target) => apps_folder_path(target),
        IconSource::Path(path) => HSTRING::from(path.as_str()),
    };
    icon::icon(&parsing_name, size)
}
