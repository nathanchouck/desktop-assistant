//! Starting an AI CLI found on this machine: a native exe, or a script run
//! by node. Shared by Claude Code and Codex, which both ship through npm
//! (a `.cmd` shim) as well as native installers.

use std::path::{Path, PathBuf};

/// How to start a CLI on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// A native executable.
    Exe(PathBuf),
    /// An npm install, started as `node <script>` rather than through its
    /// `.cmd` shim (which would go through cmd.exe and its quoting).
    Node { node: PathBuf, cli: PathBuf },
}

impl Launch {
    pub(crate) fn command(&self) -> tokio::process::Command {
        match self {
            Launch::Exe(exe) => tokio::process::Command::new(exe),
            Launch::Node { node, cli } => {
                let mut cmd = tokio::process::Command::new(node);
                cmd.arg(cli);
                cmd
            }
        }
    }
}

#[cfg(windows)]
pub(crate) const NODE: &str = "node.exe";
#[cfg(not(windows))]
pub(crate) const NODE: &str = "node";

/// The folders on PATH.
pub(crate) fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

pub(crate) fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .is_some_and(|e| extensions.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// The program an npm `.cmd` shim runs: the last quoted path relative to
/// the shim's folder (`%dp0%\...` or `%~dp0\...`) that's an exe or a script.
pub(crate) fn shim_target(shim: &str, dir: &Path) -> Option<PathBuf> {
    shim.split('"')
        .skip(1)
        .step_by(2)
        .filter_map(|quoted| {
            let rest = quoted
                .strip_prefix("%dp0%")
                .or_else(|| quoted.strip_prefix("%~dp0"))?;
            let relative = rest.trim_start_matches(['\\', '/']);
            let path = relative
                .split(['\\', '/'])
                .filter(|part| !part.is_empty())
                .fold(dir.to_path_buf(), |path, part| path.join(part));
            has_extension(&path, &["exe", "js", "mjs", "cjs"]).then_some(path)
        })
        .last()
}

/// `node` next to `near` (npm's own folder), else on PATH.
pub(crate) fn find_node(near: Option<&Path>, path_dirs: &[PathBuf]) -> Option<PathBuf> {
    near.into_iter()
        .chain(path_dirs.iter().map(PathBuf::as_path))
        .map(|dir| dir.join(NODE))
        .find(|p| p.is_file())
}

/// What an npm `.cmd` shim runs, if it can be read: a native exe or a
/// script, found on disk.
pub(crate) fn read_shim(shim: &Path) -> Option<PathBuf> {
    let dir = shim.parent()?;
    std::fs::read_to_string(shim)
        .ok()
        .and_then(|text| shim_target(&text, dir))
        .filter(|t| t.is_file())
}
