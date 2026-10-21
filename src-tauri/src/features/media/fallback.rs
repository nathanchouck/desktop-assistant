//! Other platforms: no media keys to press, nothing to watch.

pub const SUPPORTED: bool = false;

pub fn press(_vk: u16) -> Result<(), String> {
    Err("media keys work only on Windows".into())
}
