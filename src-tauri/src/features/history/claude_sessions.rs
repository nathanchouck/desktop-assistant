//! Whether Claude Code still has a session to resume. It keeps each one as
//! `<config folder>/projects/<folder>/<session id>.jsonl` and deletes those
//! unused for a while (30 days unless `cleanupPeriodDays` says otherwise);
//! after that `--resume` fails, so a reopened chat starts a new session
//! instead.

use std::path::{Path, PathBuf};

/// True when Claude Code no longer has `session`. Unsure (no config
/// folder, an id that isn't a plain UUID) counts as still there.
pub fn gone(session: &str) -> bool {
    projects_dir().is_some_and(|projects| gone_in(&projects, session))
}

/// Claude Code's config folder (`CLAUDE_CONFIG_DIR`, else `~/.claude`).
fn projects_dir() -> Option<PathBuf> {
    let config = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))?;
    Some(config.join("projects"))
}

/// True only when `projects` can be read and none of its folders has the
/// session's file.
fn gone_in(projects: &Path, session: &str) -> bool {
    // Ids are UUIDs; anything else isn't used as a file name.
    let plain = !session.is_empty()
        && session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !plain {
        return false;
    }
    let Ok(folders) = std::fs::read_dir(projects) else {
        return false;
    };
    let file = format!("{session}.jsonl");
    !folders
        .flatten()
        .any(|folder| folder.path().join(&file).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0b6c3d4e-1111-4222-8333-444455556666";

    #[test]
    fn finds_a_session_in_any_project_folder() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        let folder = projects.join("C--Users-me-Downloads");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(projects.join("C--Users-me")).unwrap();
        assert!(gone_in(&projects, ID));
        std::fs::write(folder.join(format!("{ID}.jsonl")), "{}").unwrap();
        assert!(!gone_in(&projects, ID));
    }

    #[test]
    fn unsure_means_still_there() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!gone_in(&dir.path().join("missing"), ID), "no folder");
        assert!(!gone_in(dir.path(), ""), "no id");
        assert!(!gone_in(dir.path(), "../../etc/passwd"), "not an id");
    }
}
