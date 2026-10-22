//! Updater errors in words for the footer and the settings menu.

/// An updater error in a few words, for the footer and settings.
pub fn describe(e: &tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error;
    match e {
        Error::ReleaseNotFound => "GitHub has no update information yet".into(),
        Error::Reqwest(e) if e.is_timeout() => "GitHub didn't answer in time".into(),
        Error::Reqwest(e) if e.is_connect() => "couldn't reach GitHub".into(),
        Error::Reqwest(e) => match e.status() {
            Some(status) => format!("GitHub answered {status}"),
            None => "a network error".into(),
        },
        Error::Network(message) => message.trim_matches('`').to_owned(),
        // Releases only update copies installed with the -setup.exe (NSIS).
        Error::TargetNotFound(_) | Error::TargetsNotFound(_) => {
            "the release has no update for this kind of install (use the -setup.exe)".into()
        }
        Error::Minisign(_) | Error::Base64(_) | Error::SignatureUtf8(_) => {
            "its signature doesn't match this build's key, so it wasn't used".into()
        }
        // `requireSignedVersion`: latest.json names another version than
        // the one the installer was signed for (tampered with, or stale).
        Error::SignedVersionMismatch { .. } | Error::MissingSignedVersion => {
            "it isn't signed for the version announced, so it wasn't used".into()
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_updater::Error;

    #[test]
    fn errors_in_a_few_words() {
        assert_eq!(
            describe(&Error::ReleaseNotFound),
            "GitHub has no update information yet"
        );
        assert_eq!(
            describe(&Error::Network(
                "Download request failed with status: 404 Not Found".into()
            )),
            "Download request failed with status: 404 Not Found"
        );
        assert_eq!(
            describe(&Error::TargetsNotFound(vec!["windows-x86_64".into()])),
            "the release has no update for this kind of install (use the -setup.exe)"
        );
        assert!(describe(&Error::MissingSignedVersion).contains("isn't signed"));
        assert_eq!(
            describe(&Error::EmptyEndpoints),
            "Updater does not have any endpoints set."
        );
    }
}
