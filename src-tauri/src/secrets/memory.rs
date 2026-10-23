//! Secrets in memory, for Linux dev and tests: gone when the process ends.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard};

use super::{Secret, SecretError};

static STORE: LazyLock<Mutex<HashMap<String, Secret>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn store() -> MutexGuard<'static, HashMap<String, Secret>> {
    STORE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn get(target: &str) -> Result<Option<Secret>, SecretError> {
    Ok(store().get(target).cloned())
}

pub fn set(target: &str, value: &Secret) -> Result<(), SecretError> {
    store().insert(target.to_owned(), value.clone());
    Ok(())
}

pub fn delete(target: &str) -> Result<(), SecretError> {
    store().remove(target);
    Ok(())
}
