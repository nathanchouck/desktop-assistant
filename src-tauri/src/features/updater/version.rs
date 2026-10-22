//! Comparing release versions, and judging after a restart whether an
//! install went through.

/// Same release, however it's written ("v1.2.0" and "1.2.0").
pub fn same_version(a: &str, b: &str) -> bool {
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a.trim().trim_start_matches('v') == b.trim().trim_start_matches('v'),
    }
}

/// Whether `a` is a later release than `b` (false if either isn't a version).
pub fn is_newer(a: &str, b: &str) -> bool {
    matches!((parse(a), parse(b)), (Some(a), Some(b)) if a > b)
}

fn parse(version: &str) -> Option<semver::Version> {
    semver::Version::parse(version.trim().trim_start_matches('v')).ok()
}

/// How the last install went, judged when the app starts again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfterInstall {
    /// Running the version that was being installed.
    Updated(String),
    /// Still on an older version: the installer didn't finish.
    NotInstalled(String),
}

/// `installing`: the version an install was started for before the app
/// closed, if any; `current`: the version running now.
pub fn after_install(installing: Option<&str>, current: &str) -> Option<AfterInstall> {
    let version = installing?;
    if same_version(version, current) {
        Some(AfterInstall::Updated(version.to_owned()))
    } else if is_newer(version, current) {
        Some(AfterInstall::NotInstalled(version.to_owned()))
    } else {
        // Something newer was installed some other way meanwhile.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_like_releases() {
        assert!(same_version("v1.2.0", "1.2.0"));
        assert!(!same_version("1.2.0", "1.2.1"));
        assert!(same_version("nightly", " nightly"));
        assert!(is_newer("0.10.0", "0.9.0"), "numeric, not text");
        assert!(is_newer("1.0.0", "1.0.0-beta.2"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("garbage", "1.0.0"));
    }

    #[test]
    fn after_an_install_restart() {
        assert_eq!(after_install(None, "0.2.0"), None);
        assert_eq!(
            after_install(Some("0.2.0"), "0.2.0"),
            Some(AfterInstall::Updated("0.2.0".into()))
        );
        assert_eq!(
            after_install(Some("0.2.0"), "0.1.0"),
            Some(AfterInstall::NotInstalled("0.2.0".into()))
        );
        assert_eq!(after_install(Some("0.2.0"), "0.3.0"), None);
    }
}
