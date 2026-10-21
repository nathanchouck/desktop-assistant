//! Where Codex is installed: the standalone installer, npm (whose `.cmd`
//! shim runs a native exe through node, which we start directly), WinGet,
//! or a path from the settings.

use std::path::{Path, PathBuf};

use crate::ai::launch::{find_node, has_extension, path_dirs, read_shim, Launch};

#[cfg(windows)]
const NAMES: &[&str] = &["codex.exe", "codex.cmd"];
#[cfg(not(windows))]
const NAMES: &[&str] = &["codex"];

/// Finds Codex: `override_path` if set, else PATH, else the usual install
/// locations (standalone installer, npm, WinGet).
pub fn locate(override_path: &str) -> Option<Launch> {
    locate_in(
        override_path,
        &path_dirs(),
        dirs::home_dir().as_deref(),
        dirs::data_dir().as_deref(),
    )
}

fn locate_in(
    override_path: &str,
    path_dirs: &[PathBuf],
    home: Option<&Path>,
    appdata: Option<&Path>,
) -> Option<Launch> {
    let override_path = override_path.trim();
    if !override_path.is_empty() {
        let path = PathBuf::from(override_path);
        return path.is_file().then(|| from_file(path, path_dirs)).flatten();
    }
    let mut candidates: Vec<PathBuf> = path_dirs
        .iter()
        .flat_map(|dir| NAMES.iter().map(move |name| dir.join(name)))
        .collect();
    if let Some(home) = home {
        let local = home.join("AppData").join("Local");
        // The standalone installer's bin folder, and the release it links to.
        candidates.push(local.join("Programs/OpenAI/Codex/bin").join(NAMES[0]));
        candidates.push(
            home.join(".codex/packages/standalone/current/bin")
                .join(NAMES[0]),
        );
        // WinGet's links folder: on PATH, but only for programs started
        // after the install, which the launcher (started at login) may not be.
        candidates.push(local.join("Microsoft/WinGet/Links").join(NAMES[0]));
    }
    if let Some(appdata) = appdata {
        candidates.push(appdata.join("npm").join("codex.cmd"));
    }
    candidates
        .into_iter()
        .find(|c| c.is_file())
        .and_then(|c| from_file(c, path_dirs))
}

fn from_file(path: PathBuf, path_dirs: &[PathBuf]) -> Option<Launch> {
    if has_extension(&path, &["cmd", "bat"]) {
        // npm's shim runs `node …\@openai\codex\bin\codex.js`, which starts
        // the native exe npm installed beside it. Start that exe directly:
        // through node it would open a console window of its own.
        return match read_shim(&path) {
            Some(script) if has_extension(&script, &["js", "mjs", "cjs"]) => native_beside(&script)
                .map(Launch::Exe)
                .or_else(|| {
                    find_node(path.parent(), path_dirs)
                        .map(|node| Launch::Node { node, cli: script })
                })
                .or(Some(Launch::Exe(path))),
            Some(exe) => Some(Launch::Exe(exe)),
            None => Some(Launch::Exe(path)),
        };
    }
    if has_extension(&path, &["js", "mjs", "cjs"]) {
        return native_beside(&path).map(Launch::Exe).or_else(|| {
            find_node(path.parent(), path_dirs).map(|node| Launch::Node { node, cli: path })
        });
    }
    Some(Launch::Exe(path))
}

/// `…\@openai\codex\bin\codex.js` → the native exe of the platform package
/// npm installed with it (nested for global installs, hoisted otherwise).
fn native_beside(codex_js: &Path) -> Option<PathBuf> {
    let package = codex_js.parent()?.parent()?;
    let (name, triple, exe) = platform_package();
    let inside = Path::new("vendor").join(triple).join("bin").join(exe);
    [
        package
            .join("node_modules")
            .join("@openai")
            .join(name)
            .join(&inside),
        package.parent()?.join(name).join(&inside),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// (npm package, target triple, exe name) of Codex's native build here.
fn platform_package() -> (&'static str, &'static str, &'static str) {
    if cfg!(windows) {
        if cfg!(target_arch = "aarch64") {
            ("codex-win32-arm64", "aarch64-pc-windows-msvc", "codex.exe")
        } else {
            ("codex-win32-x64", "x86_64-pc-windows-msvc", "codex.exe")
        }
    } else if cfg!(target_os = "macos") {
        ("codex-darwin-arm64", "aarch64-apple-darwin", "codex")
    } else {
        ("codex-linux-x64", "x86_64-unknown-linux-musl", "codex")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_codex_on_path_first() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let exe = bin.join(NAMES[0]);
        fs::write(&exe, b"").unwrap();
        assert_eq!(
            locate_in("", &[bin], Some(dir.path()), None),
            Some(Launch::Exe(exe))
        );
        assert_eq!(locate_in("", &[], None, None), None);
    }

    #[test]
    fn falls_back_to_the_standalone_install() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("AppData/Local/Programs/OpenAI/Codex/bin");
        fs::create_dir_all(&bin).unwrap();
        let exe = bin.join(NAMES[0]);
        fs::write(&exe, b"").unwrap();
        assert_eq!(
            locate_in("", &[], Some(dir.path()), None),
            Some(Launch::Exe(exe))
        );
    }

    /// What npm's cmd-shim writes for `@openai/codex`.
    const SHIM: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@openai\\codex\\bin\\codex.js\" %*\r\n";

    #[test]
    fn runs_the_native_exe_behind_the_npm_shim() {
        let dir = tempfile::tempdir().unwrap();
        let npm = dir.path().join("npm");
        let package = npm.join("node_modules/@openai/codex");
        fs::create_dir_all(package.join("bin")).unwrap();
        fs::write(package.join("bin/codex.js"), b"").unwrap();
        fs::write(npm.join("codex.cmd"), SHIM).unwrap();

        // Without the platform package: node runs the script.
        let node_dir = dir.path().join("nodejs");
        fs::create_dir_all(&node_dir).unwrap();
        fs::write(node_dir.join(crate::ai::launch::NODE), b"").unwrap();
        assert_eq!(
            from_file(npm.join("codex.cmd"), std::slice::from_ref(&node_dir)),
            Some(Launch::Node {
                node: node_dir.join(crate::ai::launch::NODE),
                cli: package.join("bin/codex.js"),
            })
        );

        // A global install nests the native build inside the package.
        let (name, triple, exe) = platform_package();
        let native = package
            .join("node_modules/@openai")
            .join(name)
            .join("vendor")
            .join(triple)
            .join("bin")
            .join(exe);
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        fs::write(&native, b"").unwrap();
        assert_eq!(
            from_file(npm.join("codex.cmd"), &[]),
            Some(Launch::Exe(native))
        );
    }

    #[test]
    fn finds_a_hoisted_native_build() {
        let dir = tempfile::tempdir().unwrap();
        let modules = dir.path().join("node_modules/@openai");
        let script = modules.join("codex/bin/codex.js");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(&script, b"").unwrap();
        let (name, triple, exe) = platform_package();
        let native = modules
            .join(name)
            .join("vendor")
            .join(triple)
            .join("bin")
            .join(exe);
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        fs::write(&native, b"").unwrap();
        assert_eq!(native_beside(&script), Some(native));
    }

    #[test]
    fn honors_an_override_path() {
        let dir = tempfile::tempdir().unwrap();
        let custom = dir.path().join("my-codex");
        fs::write(&custom, b"").unwrap();
        assert_eq!(
            locate_in(&custom.display().to_string(), &[], None, None),
            Some(Launch::Exe(custom))
        );
        assert_eq!(locate_in("/nope/codex", &[], None, None), None);
    }
}
