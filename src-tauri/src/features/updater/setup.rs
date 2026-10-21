//! What a build needs to update itself: the public key release signatures
//! are checked with, and where new releases are announced.
//!
//! The key is fixed when the app is built: the `TAURI_UPDATER_PUBKEY`
//! environment variable (the release workflow sets it from a repository
//! variable), else `plugins.updater.pubkey` in tauri.conf.json. A build
//! without one can't tell a real update from a forged one, so it never
//! looks for updates ("not set up").

use serde_json::{json, Value};

/// GitHub serves the newest release's files at a fixed address; the release
/// workflow uploads `latest.json` (version, installer link, signature).
pub const DEFAULT_ENDPOINT: &str =
    "https://github.com/nathanchouck/desktop-assistant/releases/latest/download/latest.json";

/// The updater's section of tauri.conf.json, completed so the plugin always
/// starts (it refuses a missing or invalid section, which would stop the
/// whole app), and the public key this build checks updates with, if any.
pub fn prepare(section: Option<&Value>, build_pubkey: Option<&str>) -> (Value, Option<String>) {
    let mut section = match section {
        Some(Value::Object(map)) => Value::Object(map.clone()),
        _ => json!({}),
    };
    let pubkey = non_empty(build_pubkey)
        .or_else(|| non_empty(section.get("pubkey").and_then(Value::as_str)));
    section["pubkey"] = pubkey.clone().unwrap_or_default().into();
    let has_endpoints = section
        .get("endpoints")
        .and_then(Value::as_array)
        .is_some_and(|list| !list.is_empty());
    if !has_endpoints {
        section["endpoints"] = json!([DEFAULT_ENDPOINT]);
    }
    // Every release is signed together with its version (the Tauri CLI
    // records it), so a tampered latest.json can't offer an older release
    // under a newer number.
    if section.get("requireSignedVersion").is_none()
        && section.get("require-signed-version").is_none()
    {
        section["requireSignedVersion"] = true.into();
    }
    if let Err(e) = serde_json::from_value::<tauri_plugin_updater::Config>(section.clone()) {
        tracing::warn!("plugins.updater in tauri.conf.json is invalid ({e}); using the defaults");
        section = json!({
            "pubkey": pubkey.clone().unwrap_or_default(),
            "endpoints": [DEFAULT_ENDPOINT],
            "requireSignedVersion": true,
        });
    }
    (section, pubkey)
}

fn non_empty(key: Option<&str>) -> Option<String> {
    key.map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_accepts(section: &Value) -> bool {
        serde_json::from_value::<tauri_plugin_updater::Config>(section.clone()).is_ok()
    }

    #[test]
    fn no_key_anywhere_is_not_set_up_but_still_starts() {
        let (section, key) = prepare(None, None);
        assert_eq!(key, None);
        assert!(plugin_accepts(&section));
        assert_eq!(section["pubkey"], "");
        assert_eq!(section["endpoints"], json!([DEFAULT_ENDPOINT]));
        assert_eq!(section["requireSignedVersion"], true);
        // An empty variable (an unset CI secret) counts as no key.
        assert_eq!(prepare(None, Some("  \n")).1, None);
    }

    #[test]
    fn the_build_variable_wins_over_the_config() {
        let config = json!({ "pubkey": "from-config" });
        assert_eq!(
            prepare(Some(&config), None).1.as_deref(),
            Some("from-config")
        );
        let (section, key) = prepare(Some(&config), Some(" from-env\n"));
        assert_eq!(key.as_deref(), Some("from-env"));
        assert_eq!(section["pubkey"], "from-env");
    }

    #[test]
    fn keeps_what_the_config_sets() {
        let config = json!({
            "pubkey": "",
            "endpoints": ["https://example.com/latest.json"],
            "requireSignedVersion": false,
            "windows": { "installMode": "quiet" },
        });
        let (section, key) = prepare(Some(&config), None);
        assert_eq!(key, None);
        assert_eq!(
            section["endpoints"],
            json!(["https://example.com/latest.json"])
        );
        assert_eq!(section["requireSignedVersion"], false);
        assert_eq!(section["windows"]["installMode"], "quiet");
        assert!(plugin_accepts(&section));
    }

    #[test]
    fn a_broken_section_falls_back_to_the_defaults() {
        for broken in [
            json!({ "pubkey": "k", "endpoints": ["not a url"] }),
            json!({ "pubkey": "k", "windows": { "installMode": "loud" } }),
            json!("not a table"),
        ] {
            let (section, key) = prepare(Some(&broken), None);
            assert!(plugin_accepts(&section), "{broken}");
            assert_eq!(section["endpoints"], json!([DEFAULT_ENDPOINT]));
            if broken.is_object() {
                assert_eq!(key.as_deref(), Some("k"));
            }
        }
    }
}
