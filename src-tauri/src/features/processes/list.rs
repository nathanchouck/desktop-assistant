//! Running processes as the rows show them: grouped by program, without
//! the ones that must never be ended. Pure, so it's tested on any OS.

use std::collections::{HashMap, HashSet};

/// One running process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub parent: u32,
    /// The program's file name as Windows reports it ("chrome.exe").
    pub exe: String,
    /// The program's full path, when Windows lets us read it.
    pub path: Option<String>,
    /// Private memory in bytes (0 when Windows doesn't let us read it).
    pub memory: u64,
    /// Titles of its visible top-level windows.
    pub windows: Vec<String>,
}

/// Processes Windows needs to keep running (ending one crashes or locks
/// the PC, or it's the Idle/System pseudo-process), plus service hosts,
/// which carry other programs' services. Lower case.
const PROTECTED: &[&str] = &[
    "[system process]",
    "system idle process",
    "system",
    "secure system",
    "registry",
    "memory compression",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "lsass.exe",
    "lsaiso.exe",
    "services.exe",
    "svchost.exe",
    "dwm.exe",
    "fontdrvhost.exe",
];

/// The Idle and System pseudo-processes.
const KERNEL_PIDS: [u32; 2] = [0, 4];

pub fn is_protected(exe: &str) -> bool {
    PROTECTED.contains(&exe.to_ascii_lowercase().as_str())
}

/// The launcher and everything it started (its web view, Claude Code):
/// ending those from the launcher would break it.
pub fn own_tree(procs: &[Proc], own_pid: u32) -> HashSet<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for p in procs {
        if p.pid != p.parent {
            children.entry(p.parent).or_default().push(p.pid);
        }
    }
    let mut tree = HashSet::from([own_pid]);
    let mut todo = vec![own_pid];
    while let Some(pid) = todo.pop() {
        for &child in children.get(&pid).into_iter().flatten() {
            // A reused PID can make the parent links loop; visit once.
            if tree.insert(child) {
                todo.push(child);
            }
        }
    }
    tree
}

/// The processes the launcher may offer to end.
pub fn killable(procs: &[Proc], own_pid: u32) -> Vec<Proc> {
    let ours = own_tree(procs, own_pid);
    procs
        .iter()
        .filter(|p| !KERNEL_PIDS.contains(&p.pid) && !is_protected(&p.exe) && !ours.contains(&p.pid))
        .cloned()
        .collect()
}

/// All running copies of one program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// As Windows spells it, from the first process seen.
    pub exe: String,
    pub procs: Vec<Proc>,
}

impl Group {
    /// Lower case, for ids and comparing.
    pub fn key(&self) -> String {
        self.exe.to_lowercase()
    }

    pub fn memory(&self) -> u64 {
        self.procs.iter().map(|p| p.memory).sum()
    }

    pub fn windows(&self) -> impl Iterator<Item = &str> {
        self.procs
            .iter()
            .flat_map(|p| p.windows.iter().map(String::as_str))
    }

    pub fn path(&self) -> Option<&str> {
        self.procs.iter().find_map(|p| p.path.as_deref())
    }

    /// "chrome.exe" without ".exe", for matching.
    pub fn stem(&self) -> &str {
        stem(&self.exe)
    }
}

pub fn stem(exe: &str) -> &str {
    match exe.len().checked_sub(4) {
        Some(cut) if exe.is_char_boundary(cut) && exe[cut..].eq_ignore_ascii_case(".exe") => {
            &exe[..cut]
        }
        _ => exe,
    }
}

/// Processes grouped by program: those with windows first (the apps you
/// see), then by memory, largest first.
pub fn groups(procs: &[Proc]) -> Vec<Group> {
    let mut by_name: HashMap<String, Group> = HashMap::new();
    for p in procs {
        by_name
            .entry(p.exe.to_lowercase())
            .or_insert_with(|| Group {
                exe: p.exe.clone(),
                procs: Vec::new(),
            })
            .procs
            .push(p.clone());
    }
    let mut groups: Vec<Group> = by_name.into_values().collect();
    groups.sort_by(|a, b| {
        let windows = |g: &Group| g.windows().next().is_some();
        windows(b)
            .cmp(&windows(a))
            .then(b.memory().cmp(&a.memory()))
            .then(a.key().cmp(&b.key()))
    });
    groups
}

/// The order to end `procs` in: each one's parent before it, so a
/// program's main process goes first and can't restart its helpers.
pub fn kill_order(procs: &[Proc]) -> Vec<u32> {
    let pids: HashSet<u32> = procs.iter().map(|p| p.pid).collect();
    let (roots, rest): (Vec<&Proc>, Vec<&Proc>) = procs
        .iter()
        .partition(|p| p.parent == p.pid || !pids.contains(&p.parent));
    roots.into_iter().chain(rest).map(|p| p.pid).collect()
}

/// The footer question before ending a program.
pub fn confirm(exe: &str, processes: usize) -> String {
    if exe.eq_ignore_ascii_case("explorer.exe") {
        return "End explorer.exe? The taskbar and desktop will disappear until Windows restarts them."
            .into();
    }
    match processes {
        1 => format!("End {exe}?"),
        n => format!("End all {n} {exe} processes?"),
    }
}

/// How ending one process went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ended,
    /// It had already exited.
    Gone,
    /// Windows refused: it runs as administrator (or another user).
    Denied,
    Failed(String),
}

/// What to tell you after ending a program's processes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    /// All gone: a toast, and the list refreshed.
    Done(String),
    /// Something's still running: a footer notice.
    Problem(String),
}

pub fn report(exe: &str, outcomes: &[Outcome]) -> Report {
    let total = outcomes.len();
    let ended = outcomes.iter().filter(|o| **o == Outcome::Ended).count();
    let gone = outcomes.iter().filter(|o| **o == Outcome::Gone).count();
    let denied = outcomes.iter().filter(|o| **o == Outcome::Denied).count();
    let failure = outcomes.iter().find_map(|o| match o {
        Outcome::Failed(e) => Some(e.as_str()),
        _ => None,
    });
    if total == 0 || ended == 0 && gone == total {
        return Report::Problem(format!("{exe} isn't running anymore."));
    }
    if ended + gone == total {
        return Report::Done(match total {
            1 => format!("Ended {exe}."),
            n => format!("Ended {n} {exe} processes."),
        });
    }
    let left = total - ended - gone;
    let done = if ended + gone == 0 {
        String::new()
    } else {
        format!("Ended {} of {total} {exe} processes. ", ended + gone)
    };
    if denied == left {
        let why = if ended + gone == 0 {
            format!("Windows didn't let the launcher end {exe}: it runs as administrator.")
        } else {
            "Windows didn't let the launcher end the rest: they run as administrator.".to_owned()
        };
        return Report::Problem(format!("{done}{why}"));
    }
    let why = failure.unwrap_or("Windows refused");
    Report::Problem(format!("{done}Couldn't end {exe}: {why}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, parent: u32, exe: &str, memory: u64, windows: &[&str]) -> Proc {
        Proc {
            pid,
            parent,
            exe: exe.into(),
            path: Some(format!(r"C:\Apps\{exe}")),
            memory,
            windows: windows.iter().map(|w| w.to_string()).collect(),
        }
    }

    fn sample() -> Vec<Proc> {
        vec![
            proc(0, 0, "[System Process]", 0, &[]),
            proc(4, 0, "System", 0, &[]),
            proc(600, 4, "csrss.exe", 1, &[]),
            proc(700, 600, "winlogon.exe", 1, &[]),
            proc(800, 700, "svchost.exe", 50, &[]),
            proc(1000, 900, "explorer.exe", 100, &["Downloads"]),
            proc(2000, 1000, "chrome.exe", 300, &["Inbox - Gmail"]),
            proc(2001, 2000, "chrome.exe", 200, &[]),
            proc(2002, 2000, "chrome.exe", 100, &[]),
            proc(3000, 1000, "Notepad.exe", 10, &["notes.txt - Notepad"]),
            proc(3001, 1000, "notepad.exe", 12, &["todo.txt - Notepad"]),
            proc(4000, 1000, "Code.exe", 900, &[]),
            // The launcher, its web view and its Claude Code.
            proc(5000, 1000, "desktop-assistant.exe", 80, &[]),
            proc(5001, 5000, "msedgewebview2.exe", 90, &[]),
            proc(5002, 5001, "msedgewebview2.exe", 40, &[]),
            proc(5003, 5000, "claude.exe", 60, &[]),
            // Someone else's web view.
            proc(6000, 4000, "msedgewebview2.exe", 70, &[]),
        ]
    }

    #[test]
    fn critical_processes_are_never_offered() {
        let pids: Vec<u32> = killable(&sample(), 5000).iter().map(|p| p.pid).collect();
        for kept_out in [0, 4, 600, 700, 800] {
            assert!(!pids.contains(&kept_out), "{kept_out}");
        }
        assert!(pids.contains(&1000), "explorer is offered (with a warning)");
        assert!(is_protected("LSASS.EXE"));
        assert!(is_protected("System"));
        assert!(!is_protected("chrome.exe"));
    }

    #[test]
    fn the_launcher_and_its_children_are_never_offered() {
        let pids: Vec<u32> = killable(&sample(), 5000).iter().map(|p| p.pid).collect();
        for ours in [5000, 5001, 5002, 5003] {
            assert!(!pids.contains(&ours), "{ours}");
        }
        assert!(pids.contains(&6000), "another app's web view is fine");
    }

    #[test]
    fn parent_loops_dont_hang() {
        let looped = vec![proc(1, 2, "a.exe", 0, &[]), proc(2, 1, "b.exe", 0, &[])];
        assert_eq!(own_tree(&looped, 1), HashSet::from([1, 2]));
        let own_parent = vec![proc(7, 7, "a.exe", 0, &[])];
        assert_eq!(own_tree(&own_parent, 7), HashSet::from([7]));
    }

    #[test]
    fn groups_by_program_ignoring_case() {
        let groups = groups(&killable(&sample(), 5000));
        let notepad = groups.iter().find(|g| g.key() == "notepad.exe").unwrap();
        assert_eq!(notepad.procs.len(), 2);
        assert_eq!(notepad.exe, "Notepad.exe");
        assert_eq!(notepad.memory(), 22);
        assert_eq!(
            notepad.windows().collect::<Vec<_>>(),
            ["notes.txt - Notepad", "todo.txt - Notepad"]
        );
        let chrome = groups.iter().find(|g| g.key() == "chrome.exe").unwrap();
        assert_eq!((chrome.procs.len(), chrome.memory()), (3, 600));
        assert_eq!(chrome.stem(), "chrome");
        assert_eq!(chrome.path(), Some(r"C:\Apps\chrome.exe"));
    }

    #[test]
    fn apps_with_windows_come_first_then_memory() {
        let groups = groups(&killable(&sample(), 5000));
        let order: Vec<String> = groups.iter().map(Group::key).collect();
        assert_eq!(
            order,
            [
                "chrome.exe",
                "explorer.exe",
                "notepad.exe",
                "code.exe",
                "msedgewebview2.exe"
            ]
        );
    }

    #[test]
    fn parents_are_ended_first() {
        let chrome = vec![
            proc(2002, 2000, "chrome.exe", 0, &[]),
            proc(2000, 1000, "chrome.exe", 0, &[]),
            proc(2001, 2000, "chrome.exe", 0, &[]),
        ];
        assert_eq!(kill_order(&chrome), [2000, 2002, 2001]);
    }

    #[test]
    fn stems() {
        assert_eq!(stem("chrome.exe"), "chrome");
        assert_eq!(stem("WINWORD.EXE"), "WINWORD");
        assert_eq!(stem("System"), "System");
        assert_eq!(stem("é.exe"), "é");
        assert_eq!(stem("exe"), "exe");
    }

    #[test]
    fn questions() {
        assert_eq!(confirm("chrome.exe", 12), "End all 12 chrome.exe processes?");
        assert_eq!(confirm("notepad.exe", 1), "End notepad.exe?");
        assert!(confirm("Explorer.EXE", 1).contains("taskbar"));
    }

    #[test]
    fn reports() {
        use Outcome::*;
        assert_eq!(
            report("chrome.exe", &[Ended, Ended, Gone]),
            Report::Done("Ended 3 chrome.exe processes.".into())
        );
        assert_eq!(
            report("notepad.exe", &[Ended]),
            Report::Done("Ended notepad.exe.".into())
        );
        assert_eq!(
            report("chrome.exe", &[Gone, Gone]),
            Report::Problem("chrome.exe isn't running anymore.".into())
        );
        assert_eq!(
            report("x.exe", &[]),
            Report::Problem("x.exe isn't running anymore.".into())
        );
        assert_eq!(
            report("Taskmgr.exe", &[Denied]),
            Report::Problem(
                "Windows didn't let the launcher end Taskmgr.exe: it runs as administrator."
                    .into()
            )
        );
        assert_eq!(
            report("chrome.exe", &[Ended, Ended, Denied]),
            Report::Problem(
                "Ended 2 of 3 chrome.exe processes. Windows didn't let the launcher end the rest: they run as administrator."
                    .into()
            )
        );
        assert_eq!(
            report("app.exe", &[Ended, Failed("The handle is invalid.".into())]),
            Report::Problem(
                "Ended 1 of 2 app.exe processes. Couldn't end app.exe: The handle is invalid."
                    .into()
            )
        );
    }
}
