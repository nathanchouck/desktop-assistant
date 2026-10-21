//! Other platforms: nothing to pick from (the app is Windows-only; this
//! keeps it building and testable elsewhere).

use std::path::Path;
use std::time::Duration;

use super::model::Rgb;

pub const SUPPORTED: bool = false;

pub fn pick(_timeout: Duration, _marker: &Path) -> Result<Option<Rgb>, String> {
    Err("only on Windows".into())
}

pub fn recover(_marker: &Path) {}
