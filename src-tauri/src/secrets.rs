//! Named secrets (API keys, MCP server tokens), kept out of the settings
//! files. On Windows they are generic credentials in Credential Manager,
//! named `desktop-assistant:<name>` and stored for this PC only (never
//! roamed, never in the synced `settings.toml`). Elsewhere (Linux dev) and
//! in every test run they live in memory for the life of the process, so
//! tests never touch a real store.
//!
//! Values are never logged: `Secret`'s `Debug` prints `***`, and errors
//! name the secret, never its value.

#[cfg(any(test, not(windows)))]
mod memory;
#[cfg(windows)]
mod win;

#[cfg(any(test, not(windows)))]
use memory as backend;
#[cfg(all(windows, not(test)))]
use win as backend;

use std::fmt;

/// Claude's API key, for API key billing.
pub const ANTHROPIC_API_KEY: &str = "anthropic-api-key";
/// ChatGPT's (OpenAI's) API key, for API key billing.
pub const OPENAI_API_KEY: &str = "openai-api-key";

/// In front of every credential the launcher writes, so its entries are
/// easy to find (and tell apart) in Credential Manager.
const TARGET_PREFIX: &str = "desktop-assistant:";
/// The most Credential Manager keeps for one credential
/// (`CRED_MAX_CREDENTIAL_BLOB_SIZE`). Values are stored as UTF-8.
pub const MAX_BYTES: usize = 2560;
const MAX_NAME: usize = 200;
/// Values shorter than this don't show their last four characters, which
/// would give too much of them away.
const HINT_MIN_CHARS: usize = 12;

/// A secret value. Its `Debug` never shows it, and it has no `Display` or
/// `Serialize`, so it can't end up in a log or an IPC reply by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value itself, for the one place that hands it over.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The last four characters, to tell saved keys apart. `None` for
    /// short values.
    pub fn last4(&self) -> Option<String> {
        let chars: Vec<char> = self.0.chars().collect();
        (chars.len() >= HINT_MIN_CHARS).then(|| chars[chars.len() - 4..].iter().collect())
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("\"{0}\" can't be used as a secret's name")]
    BadName(String),
    #[error("the value is empty")]
    Empty,
    #[error("the value is too long to keep in Credential Manager (at most {MAX_BYTES} bytes)")]
    TooLong,
    /// What went wrong, without the value.
    #[error("Credential Manager: {0}")]
    Store(String),
}

/// The saved value of `name`, if there is one.
pub fn get(name: &str) -> Result<Option<Secret>, SecretError> {
    check_name(name)?;
    backend::get(&target(name))
}

/// `get` for callers that carry on without the secret: a failure is
/// logged (by name) and counts as not saved.
pub fn read(name: &str) -> Option<Secret> {
    get(name).unwrap_or_else(|e| {
        tracing::warn!("could not read the secret {name}: {e}");
        None
    })
}

/// Saves `value` as `name`, replacing any saved one.
pub fn set(name: &str, value: &Secret) -> Result<(), SecretError> {
    check_name(name)?;
    check_value(value.expose())?;
    backend::set(&target(name), value)
}

/// Forgets `name`. Forgetting one that isn't saved is fine.
pub fn delete(name: &str) -> Result<(), SecretError> {
    check_name(name)?;
    backend::delete(&target(name))
}

/// The credential's name in Credential Manager.
pub fn target(name: &str) -> String {
    format!("{TARGET_PREFIX}{name}")
}

/// Names are plain ASCII words, so they read well in Credential Manager:
/// letters, digits and `:_.-`.
pub fn check_name(name: &str) -> Result<(), SecretError> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '.' | '-'));
    if ok {
        Ok(())
    } else {
        Err(SecretError::BadName(name.to_owned()))
    }
}

fn check_value(value: &str) -> Result<(), SecretError> {
    if value.is_empty() {
        Err(SecretError::Empty)
    } else if value.len() > MAX_BYTES {
        Err(SecretError::TooLong)
    } else {
        Ok(())
    }
}

/// A stored value as text. The launcher writes UTF-8; a credential added
/// by hand (`cmdkey /generic:… /pass:…`, PowerShell) is UTF-16, which is
/// told apart by its zero bytes (no key or token contains one).
pub fn decode_blob(bytes: &[u8]) -> Option<String> {
    if bytes.contains(&0) {
        if bytes.len() % 2 != 0 {
            return None;
        }
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16(&units)
            .ok()
            .map(|s| s.trim_end_matches('\0').to_owned())
    } else {
        String::from_utf8(bytes.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_in_memory() {
        let name = "test-round-trip";
        assert!(get(name).unwrap().is_none());
        set(name, &Secret::new("sk-ant-one")).unwrap();
        assert_eq!(get(name).unwrap().unwrap().expose(), "sk-ant-one");
        set(name, &Secret::new("sk-ant-two")).unwrap();
        assert_eq!(read(name).unwrap().expose(), "sk-ant-two");
        delete(name).unwrap();
        assert!(get(name).unwrap().is_none());
        delete(name).unwrap();
    }

    #[test]
    fn never_shows_the_value() {
        let secret = Secret::new("sk-ant-api03-very-secret-value");
        assert_eq!(format!("{secret:?}"), "***");
        assert_eq!(format!("{:?}", Some(secret.clone())), "Some(***)");
        assert_eq!(format!("{:#?}", vec![secret]), "[\n    ***,\n]");
        let err = set("test-empty", &Secret::new("")).unwrap_err();
        assert!(matches!(err, SecretError::Empty));
    }

    #[test]
    fn shows_only_the_last_four_of_long_values() {
        assert_eq!(
            Secret::new("sk-proj-abcdefgh1234").last4().as_deref(),
            Some("1234")
        );
        assert_eq!(Secret::new("short-pass").last4(), None);
        assert_eq!(Secret::new("ключ-ключ-ключ").last4().as_deref(), Some("ключ"));
    }

    #[test]
    fn checks_names_and_sizes() {
        for good in [
            ANTHROPIC_API_KEY,
            OPENAI_API_KEY,
            "mcp:github:env:GITHUB_TOKEN",
            "mcp:linear:header:Authorization",
        ] {
            assert!(check_name(good).is_ok(), "{good}");
        }
        for bad in ["", "has space", "slash/name", "ü", &"x".repeat(201)] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
        assert!(matches!(
            set("test-too-long", &Secret::new("x".repeat(MAX_BYTES + 1))),
            Err(SecretError::TooLong)
        ));
        assert!(set("test-just-fits", &Secret::new("x".repeat(MAX_BYTES))).is_ok());
        assert_eq!(
            target(ANTHROPIC_API_KEY),
            "desktop-assistant:anthropic-api-key"
        );
    }

    #[test]
    fn reads_utf8_and_utf16_values() {
        assert_eq!(decode_blob(b"sk-123").as_deref(), Some("sk-123"));
        let utf16: Vec<u8> = "sk-123".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode_blob(&utf16).as_deref(), Some("sk-123"));
        let with_nul: Vec<u8> = "sk\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode_blob(&with_nul).as_deref(), Some("sk"));
        assert_eq!(decode_blob(&[0x41, 0x00, 0x42]), None, "odd UTF-16");
        assert_eq!(decode_blob(&[0xff, 0xfe, 0xfd]), None, "not text");
        assert_eq!(decode_blob(b"").as_deref(), Some(""));
    }

    /// Writes, reads and deletes a real credential. Run by hand on Windows:
    /// `cargo test secrets -- --ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore = "touches the real Credential Manager"]
    fn round_trips_in_credential_manager() {
        let target = target("test-credential-manager");
        win::set(&target, &Secret::new("sk-test-value-1234")).unwrap();
        assert_eq!(
            win::get(&target).unwrap().unwrap().expose(),
            "sk-test-value-1234"
        );
        win::delete(&target).unwrap();
        assert!(win::get(&target).unwrap().is_none());
        win::delete(&target).unwrap();
    }
}
