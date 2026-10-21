//! The kinds of script we run, and finding the program that runs each:
//! Windows PowerShell, Python (`py -3`, else `python`), cmd for batch
//! files, Node.js, and Git Bash.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::header::CommentStyle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    PowerShell,
    Python,
    Batch,
    Node,
    Bash,
}

impl Language {
    /// By file extension; `None` for files that aren't scripts we run.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "ps1" => Self::PowerShell,
            "py" => Self::Python,
            "cmd" | "bat" => Self::Batch,
            "js" => Self::Node,
            "sh" => Self::Bash,
            _ => return None,
        })
    }

    pub fn comment_style(self) -> CommentStyle {
        match self {
            Self::PowerShell | Self::Python | Self::Bash => CommentStyle::Hash,
            Self::Batch => CommentStyle::Batch,
            Self::Node => CommentStyle::Slashes,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PowerShell => "PowerShell",
            Self::Python => "Python",
            Self::Batch => "Batch",
            Self::Node => "Node.js",
            Self::Bash => "Bash",
        }
    }

    /// Why a script in this language can't run here.
    pub fn missing(self) -> &'static str {
        match self {
            Self::PowerShell => "Windows PowerShell wasn't found.",
            Self::Python => "Python isn't installed (no py or python on PATH).",
            Self::Batch => "Batch files only run on Windows.",
            Self::Node => "Node.js isn't installed (no node on PATH).",
            Self::Bash => "Bash scripts need Git Bash, which wasn't found.",
        }
    }
}

/// How to start a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Runner {
    /// A program given the script's path, after these arguments.
    Program { program: PathBuf, args: Vec<String> },
    /// The script itself: batch files, which Rust's standard library hands
    /// to `cmd.exe /d /c` with cmd's own quoting rules applied to the
    /// argument (a plain `cmd /C` would let `&` or `|` in typed text run
    /// as commands).
    Itself,
}

/// The command line for `script` with an optional text `argument`:
/// (program, arguments).
pub fn command_line(
    runner: &Runner,
    script: &Path,
    argument: Option<&str>,
) -> (PathBuf, Vec<OsString>) {
    let (program, mut args) = match runner {
        Runner::Program { program, args } => {
            let mut all: Vec<OsString> = args.iter().map(OsString::from).collect();
            all.push(script.as_os_str().to_owned());
            (program.clone(), all)
        }
        Runner::Itself => (script.to_path_buf(), Vec::new()),
    };
    args.extend(argument.map(OsString::from));
    (program, args)
}

/// The programs found on this machine, looked up once per scan of the
/// scripts folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Interpreters {
    pub powershell: Option<Runner>,
    pub python: Option<Runner>,
    pub batch: Option<Runner>,
    pub node: Option<Runner>,
    pub bash: Option<Runner>,
}

impl Interpreters {
    pub fn runner(&self, language: Language) -> Option<&Runner> {
        match language {
            Language::PowerShell => self.powershell.as_ref(),
            Language::Python => self.python.as_ref(),
            Language::Batch => self.batch.as_ref(),
            Language::Node => self.node.as_ref(),
            Language::Bash => self.bash.as_ref(),
        }
    }

    /// Looks on PATH and in the usual install folders (the launcher's
    /// PATH is from when you signed in, so a fresh install may be missing
    /// from it).
    pub fn find() -> Self {
        let places = Places::from_env();
        Self::find_in(&places)
    }

    fn find_in(places: &Places) -> Self {
        let program = |program: PathBuf, args: &[&str]| Runner::Program {
            program,
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        };
        if cfg!(windows) {
            Self {
                powershell: windows_powershell(places).map(|p| {
                    program(p, &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
                }),
                python: find(&["py.exe"], places, &places.python_dirs())
                    .map(|p| program(p, &["-3"]))
                    .or_else(|| find(&["python.exe"], places, &[]).map(|p| program(p, &[]))),
                batch: Some(Runner::Itself),
                node: find(&["node.exe"], places, &places.node_dirs()).map(|p| program(p, &[])),
                bash: git_bash(places).map(|p| program(p, &[])),
            }
        } else {
            // For trying things out while developing on Linux.
            Self {
                powershell: find(&["pwsh"], places, &[])
                    .map(|p| program(p, &["-NoProfile", "-NonInteractive", "-File"])),
                python: find(&["python3", "python"], places, &[]).map(|p| program(p, &[])),
                batch: None,
                node: find(&["node"], places, &[]).map(|p| program(p, &[])),
                bash: find(&["bash"], places, &[]).map(|p| program(p, &[])),
            }
        }
    }
}

/// Where programs are looked for.
#[derive(Debug, Clone, Default)]
struct Places {
    path: Vec<PathBuf>,
    system_root: Option<PathBuf>,
    program_files: Vec<PathBuf>,
    local_app_data: Option<PathBuf>,
}

impl Places {
    fn from_env() -> Self {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            path: crate::ai::launch::path_dirs(),
            system_root: var("SystemRoot"),
            program_files: ["ProgramFiles", "ProgramW6432"]
                .into_iter()
                .filter_map(var)
                .collect(),
            local_app_data: var("LOCALAPPDATA"),
        }
    }

    /// Where the Python launcher (`py.exe`) installs: for everyone, or
    /// just for you.
    fn python_dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = self.system_root.iter().cloned().collect();
        dirs.extend(
            self.local_app_data
                .iter()
                .map(|d| d.join(r"Programs\Python\Launcher")),
        );
        dirs
    }

    fn node_dirs(&self) -> Vec<PathBuf> {
        self.program_files.iter().map(|d| d.join("nodejs")).collect()
    }
}

/// The first of `names` on PATH, else in `extra` folders. Earlier names
/// win over later ones wherever they are.
fn find(names: &[&str], places: &Places, extra: &[PathBuf]) -> Option<PathBuf> {
    names.iter().find_map(|name| {
        places
            .path
            .iter()
            .chain(extra)
            .map(|dir| dir.join(name))
            .find(|p| exists(p))
    })
}

/// Also true for app execution aliases (WindowsApps\python.exe), which
/// are reparse points that don't open like normal files.
fn exists(path: &Path) -> bool {
    path.is_file() || std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_dir())
}

/// Windows PowerShell 5.1, which every Windows has: its fixed place,
/// else on PATH.
fn windows_powershell(places: &Places) -> Option<PathBuf> {
    places
        .system_root
        .iter()
        .map(|root| root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
        .find(|p| exists(p))
        .or_else(|| find(&["powershell.exe"], places, &[]))
}

/// `bash.exe` in System32 (and its WindowsApps alias) starts WSL, which
/// can't run a script by its Windows path, so it never counts.
pub fn is_wsl_launcher(path: &Path) -> bool {
    // Split by hand so Windows paths read the same in tests on Linux.
    let text = path.to_string_lossy();
    let dir = text.rsplit(['\\', '/']).nth(1).unwrap_or("");
    dir.eq_ignore_ascii_case("System32") || dir.eq_ignore_ascii_case("WindowsApps")
}

/// Git Bash: `bash.exe` on PATH (but not WSL's), else next to `git.exe`
/// on PATH (Git's installer puts only `Git\cmd` there), else Git's usual
/// install folders.
fn git_bash(places: &Places) -> Option<PathBuf> {
    let on_path = places
        .path
        .iter()
        .map(|dir| dir.join("bash.exe"))
        .find(|p| exists(p) && !is_wsl_launcher(p));
    on_path.or_else(|| {
        git_bash_candidates(places)
            .into_iter()
            .find(|p| exists(p))
    })
}

fn git_bash_candidates(places: &Places) -> Vec<PathBuf> {
    // `Git\cmd\git.exe` → `Git\bin\bash.exe`.
    let beside_git = places
        .path
        .iter()
        .filter(|dir| dir.join("git.exe").is_file())
        .filter_map(|dir| dir.parent().map(|git| git.join(r"bin\bash.exe")));
    let per_user = places.local_app_data.as_ref().map(|d| d.join("Programs"));
    let installed = places
        .program_files
        .iter()
        .chain(&per_user)
        .map(|dir| dir.join(r"Git\bin\bash.exe"));
    beside_git.chain(installed).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_by_extension() {
        let lang = |name: &str| Language::from_path(Path::new(name));
        assert_eq!(lang("a.ps1"), Some(Language::PowerShell));
        assert_eq!(lang("A.PY"), Some(Language::Python));
        assert_eq!(lang("x.cmd"), Some(Language::Batch));
        assert_eq!(lang("x.Bat"), Some(Language::Batch));
        assert_eq!(lang("x.js"), Some(Language::Node));
        assert_eq!(lang("x.sh"), Some(Language::Bash));
        assert_eq!(lang("README.txt"), None);
        assert_eq!(lang("noext"), None);
        assert_eq!(Language::Batch.comment_style(), CommentStyle::Batch);
        assert_eq!(Language::Node.comment_style(), CommentStyle::Slashes);
        assert_eq!(Language::Python.comment_style(), CommentStyle::Hash);
    }

    #[test]
    fn command_lines() {
        let ps = Runner::Program {
            program: "powershell.exe".into(),
            args: vec!["-File".into()],
        };
        let script = Path::new("hello.ps1");
        assert_eq!(
            command_line(&ps, script, Some("Ana & co")),
            (
                PathBuf::from("powershell.exe"),
                vec!["-File".into(), "hello.ps1".into(), "Ana & co".into()]
            )
        );
        assert_eq!(
            command_line(&ps, script, None).1,
            vec![OsString::from("-File"), "hello.ps1".into()]
        );
        // Batch files are started as themselves; std adds cmd.exe.
        assert_eq!(
            command_line(&Runner::Itself, Path::new("x.cmd"), Some("a")),
            (PathBuf::from("x.cmd"), vec![OsString::from("a")])
        );
    }

    #[test]
    fn wsl_bash_never_counts() {
        assert!(is_wsl_launcher(Path::new(r"C:\Windows\System32\bash.exe")));
        assert!(is_wsl_launcher(Path::new("C:/Windows/system32/bash.exe")));
        assert!(is_wsl_launcher(Path::new(
            "C:/Users/me/AppData/Local/Microsoft/WindowsApps/bash.exe"
        )));
        assert!(!is_wsl_launcher(Path::new(
            "C:/Program Files/Git/bin/bash.exe"
        )));
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }

    #[test]
    fn git_bash_is_found_beside_git() {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join("Git");
        touch(&git.join("cmd").join("git.exe"));
        let places = Places {
            path: vec![git.join("cmd")],
            ..Places::default()
        };
        // The candidate path uses Windows separators inside the folder.
        let candidates = git_bash_candidates(&places);
        assert_eq!(candidates, [git.join(r"bin\bash.exe")]);
        let places = Places {
            program_files: vec![dir.path().join("PF")],
            local_app_data: Some(dir.path().join("Local")),
            ..Places::default()
        };
        assert_eq!(
            git_bash_candidates(&places),
            [
                dir.path().join("PF").join(r"Git\bin\bash.exe"),
                dir.path().join("Local").join("Programs").join(r"Git\bin\bash.exe"),
            ]
        );
    }

    #[test]
    fn earlier_names_win_and_extra_folders_are_a_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        touch(&a.join("python"));
        touch(&b.join("py"));
        let places = Places {
            path: vec![a.clone()],
            ..Places::default()
        };
        assert_eq!(find(&["py", "python"], &places, &[]), Some(a.join("python")));
        assert_eq!(
            find(&["py", "python"], &places, std::slice::from_ref(&b)),
            Some(b.join("py"))
        );
        assert_eq!(find(&["node"], &places, &[]), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn finds_programs_on_path_off_windows() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("bash"));
        touch(&dir.path().join("python3"));
        let found = Interpreters::find_in(&Places {
            path: vec![dir.path().to_path_buf()],
            ..Places::default()
        });
        assert_eq!(
            found.bash,
            Some(Runner::Program {
                program: dir.path().join("bash"),
                args: vec![]
            })
        );
        assert!(found.python.is_some());
        assert_eq!(found.node, None);
        assert_eq!(found.batch, None);
        assert_eq!(found.runner(Language::Bash), found.bash.as_ref());
    }
}
