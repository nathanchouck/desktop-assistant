//! Other platforms: no processes to list or end (the app is Windows-only;
//! this keeps it building and testable elsewhere).

use super::list::{Outcome, Proc};

pub const SUPPORTED: bool = false;

pub fn snapshot() -> Result<Vec<Proc>, String> {
    Ok(Vec::new())
}

pub fn end(pids: &[u32]) -> Vec<Outcome> {
    pids.iter()
        .map(|_| Outcome::Failed("only on Windows".into()))
        .collect()
}
