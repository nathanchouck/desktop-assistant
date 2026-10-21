//! What a script printed, turned into what the launcher shows: capped
//! buffers, decoding, the line worth showing, and the messages for a
//! failure or a time-out. Pure.

use std::time::Duration;

/// What a script printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Finished {
        /// The exit code; `None` when it was killed by a signal (not on Windows).
        code: Option<i32>,
        stdout: String,
        stderr: String,
        /// Output went past the cap and was cut.
        cut: bool,
    },
    TimedOut {
        after: Duration,
    },
    /// The program couldn't be started.
    NotStarted(String),
}

/// Messages in the footer are cut to this many characters.
const MESSAGE_CHARS: usize = 200;
/// Output this short (on one line, or joined with " · ") shows in the
/// footer; longer output opens as a text file.
pub const NOTICE_CHARS: usize = 160;

/// Bytes kept from a stream: the start (stdout, which is the result) or
/// the end (stderr, where the error is).
#[derive(Debug)]
pub struct Capped {
    bytes: Vec<u8>,
    cap: usize,
    keep_end: bool,
    cut: bool,
}

impl Capped {
    pub fn start(cap: usize) -> Self {
        Self {
            bytes: Vec::new(),
            cap,
            keep_end: false,
            cut: false,
        }
    }

    pub fn end(cap: usize) -> Self {
        Self {
            keep_end: true,
            ..Self::start(cap)
        }
    }

    pub fn push(&mut self, chunk: &[u8]) {
        if self.keep_end {
            self.bytes.extend_from_slice(chunk);
            if self.bytes.len() > self.cap {
                let extra = self.bytes.len() - self.cap;
                self.bytes.drain(..extra);
                self.cut = true;
            }
        } else {
            let room = self.cap.saturating_sub(self.bytes.len());
            if chunk.len() > room {
                self.cut = true;
            }
            self.bytes
                .extend_from_slice(&chunk[..chunk.len().min(room)]);
        }
    }

    /// The text, and whether some was cut.
    pub fn text(&self) -> (String, bool) {
        let mut bytes = self.bytes.as_slice();
        if self.cut && self.keep_end {
            // The cut may have split a character: drop its leftover bytes.
            let partial = bytes
                .iter()
                .take(3)
                .take_while(|b| (**b & 0b1100_0000) == 0b1000_0000)
                .count();
            bytes = &bytes[partial..];
        }
        (decode(bytes), self.cut)
    }
}

/// Programs print UTF-8 (Python with `PYTHONIOENCODING`, Node, Git Bash)
/// or, on Windows, the console's OEM code page (cmd's `echo`, Windows
/// PowerShell). Text that isn't valid UTF-8 is read as the latter.
pub fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        // Only cut short at the end (the cap split a character).
        Err(e) if e.error_len().is_none() => {
            String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned()
        }
        Err(_) => oem(bytes).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned()),
    }
}

#[cfg(windows)]
fn oem(bytes: &[u8]) -> Option<String> {
    super::win::oem_to_string(bytes)
}

#[cfg(not(windows))]
fn oem(_bytes: &[u8]) -> Option<String> {
    None
}

/// The script's result text: its output without the line break(s) at the
/// end, or why it failed.
pub fn result(title: &str, outcome: &Outcome) -> Result<String, String> {
    match outcome {
        Outcome::Finished {
            code: Some(0),
            stdout,
            ..
        } => Ok(stdout.trim_end_matches(['\r', '\n']).to_owned()),
        Outcome::Finished {
            code,
            stdout,
            stderr,
            ..
        } => {
            let line = error_line(stderr).or_else(|| error_line(stdout));
            Err(match (line, code) {
                (Some(line), _) => format!("{title} failed: {line}"),
                (None, Some(code)) => format!("{title} failed (exit code {code})."),
                (None, None) => format!("{title} was stopped."),
            })
        }
        Outcome::TimedOut { after } => Err(format!(
            "{title} took longer than {} s and was stopped.",
            after.as_secs()
        )),
        Outcome::NotStarted(reason) => Err(format!("Couldn't start {title}: {reason}")),
    }
    .map_err(|message| shorten(&message, MESSAGE_CHARS))
}

/// The toast for a silent script that worked: the last line it printed.
pub fn last_line(title: &str, stdout: &str) -> String {
    match stdout.lines().map(str::trim).rfind(|l| !l.is_empty()) {
        Some(line) => shorten(line, MESSAGE_CHARS),
        None => format!("Ran {title}."),
    }
}

/// Output short enough for the footer, on one line; `None` when it needs
/// more room (or is empty).
pub fn as_notice(output: &str) -> Option<String> {
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let joined = lines.join(" · ");
    (!joined.is_empty() && joined.chars().count() <= NOTICE_CHARS).then_some(joined)
}

/// The line of an error report that says what went wrong. Skips the
/// decoration around it: PowerShell's position and `+ CategoryInfo`
/// lines, JavaScript stack frames, Node's version line, carets.
pub fn error_line(text: &str) -> Option<String> {
    powershell_error(text).or_else(|| {
        let lines: Vec<&str> = text.lines().collect();
        let last = lines.iter().rposition(|l| !is_noise(l))?;
        // cmd breaks its "is not recognized" message after a comma.
        let line = match last.checked_sub(1).map(|i| lines[i].trim()) {
            Some(before) if before.ends_with(',') => {
                format!("{before} {}", lines[last].trim())
            }
            _ => lines[last].trim().to_owned(),
        };
        Some(shorten(&line, MESSAGE_CHARS))
    })
}

/// Windows PowerShell reports an error as its message (wrapped over
/// several lines when long), then `At <file>:<line> char:<n>`, then
/// details. The message is the lines before the last such position line.
fn powershell_error(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let at = lines
        .iter()
        .rposition(|l| is_powershell_position(l.trim()))?;
    let start = lines[..at]
        .iter()
        .rposition(|l| is_noise(l))
        .map_or(0, |i| i + 1);
    let message = lines[start..at]
        .iter()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join(" ");
    (!message.is_empty()).then(|| shorten(&message, MESSAGE_CHARS))
}

fn is_powershell_position(line: &str) -> bool {
    line.starts_with("At ") && (line.contains(" char:") || line.starts_with("At line:"))
}

fn is_noise(line: &str) -> bool {
    let t = line.trim();
    t.is_empty()
        || t.starts_with('+')
        || t.chars().all(|c| matches!(c, '^' | '~' | ' '))
        || (t.starts_with("at ") && line.starts_with(char::is_whitespace))
        || t.starts_with("Node.js v")
        || is_powershell_position(t)
}

fn shorten(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max_chars - 1).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(code: i32, stdout: &str, stderr: &str) -> Outcome {
        Outcome::Finished {
            code: Some(code),
            stdout: stdout.into(),
            stderr: stderr.into(),
            cut: false,
        }
    }

    #[test]
    fn keeps_the_start_or_the_end() {
        let mut start = Capped::start(5);
        start.push(b"abc");
        start.push(b"defgh");
        assert_eq!(start.text(), ("abcde".into(), true));

        let mut end = Capped::end(5);
        end.push(b"abc");
        assert_eq!(end.text(), ("abc".into(), false));
        end.push(b"defgh");
        assert_eq!(end.text(), ("defgh".into(), true));
    }

    #[test]
    fn a_cut_never_splits_a_character() {
        // "é" is two bytes; keeping the last 3 of "aébc"... starts mid-é.
        let mut end = Capped::end(3);
        end.push("aébc".as_bytes());
        assert_eq!(end.text().0, "bc");
        let mut start = Capped::start(2);
        start.push("aé".as_bytes());
        assert_eq!(start.text().0, "a");
    }

    #[test]
    fn decodes_utf8() {
        assert_eq!(decode("héllo 👋".as_bytes()), "héllo 👋");
        // Not UTF-8 and no code page to try (off Windows): replaced, not lost.
        #[cfg(not(windows))]
        assert_eq!(decode(b"caf\x82"), "caf\u{FFFD}");
    }

    #[test]
    fn success_is_the_output_without_its_last_line_break() {
        assert_eq!(
            result("T", &finished(0, "a\r\nb\r\n\r\n", "")),
            Ok("a\r\nb".into())
        );
        // Warnings on stderr don't make it a failure.
        assert_eq!(
            result("T", &finished(0, "ok\n", "warning\n")),
            Ok("ok".into())
        );
    }

    #[test]
    fn failures_say_why() {
        assert_eq!(
            result("Backup", &finished(2, "", "copying\nerror: disk full\n\n")),
            Err("Backup failed: error: disk full".into())
        );
        assert_eq!(
            result("Backup", &finished(1, "nothing on stderr\n", "")),
            Err("Backup failed: nothing on stderr".into())
        );
        assert_eq!(
            result("Backup", &finished(3, "", "")),
            Err("Backup failed (exit code 3).".into())
        );
        assert_eq!(
            result(
                "Backup",
                &Outcome::TimedOut {
                    after: Duration::from_secs(60)
                }
            ),
            Err("Backup took longer than 60 s and was stopped.".into())
        );
        assert_eq!(
            result("Backup", &Outcome::NotStarted("access denied".into())),
            Err("Couldn't start Backup: access denied".into())
        );
        let long = result("T", &finished(1, "", &"x".repeat(500))).unwrap_err();
        assert_eq!(long.chars().count(), MESSAGE_CHARS);
    }

    #[test]
    fn powershell_errors() {
        let thrown = "Cannot find the backup drive\r\nAt C:\\Users\\me\\scripts\\backup.ps1:3 char:1\r\n\
                      + throw \"Cannot find the backup drive\"\r\n+ ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~\r\n\
                      \x20   + CategoryInfo          : OperationStopped: (Cannot find the backup drive:String) [], RuntimeException\r\n\
                      \x20   + FullyQualifiedErrorId : Cannot find the backup drive\r\n";
        assert_eq!(
            error_line(thrown).as_deref(),
            Some("Cannot find the backup drive")
        );
        // A long message wrapped over two lines comes back as one.
        let wrapped = "Get-Item : Cannot find path 'C:\\nope' because it does\nnot exist.\n\
                       At line:1 char:1\n+ Get-Item C:\\nope\n+ ~~~~~~~~~~~~~~~~\n";
        assert_eq!(
            error_line(wrapped).as_deref(),
            Some("Get-Item : Cannot find path 'C:\\nope' because it does not exist.")
        );
    }

    #[test]
    fn python_node_and_cmd_errors() {
        let python = "Traceback (most recent call last):\n  File \"x.py\", line 2, in <module>\n\
                      \x20   int('a')\n    ~~~^^^^^\nValueError: invalid literal for int() with base 10: 'a'\n";
        assert_eq!(
            error_line(python).as_deref(),
            Some("ValueError: invalid literal for int() with base 10: 'a'")
        );
        let node = "C:\\s\\x.js:1\nthrow new Error('boom')\n^\n\nError: boom\n    at Object.<anonymous> (C:\\s\\x.js:1:7)\n\
                    \x20   at Module._compile (node:internal/modules/cjs/loader:1554:14)\n\nNode.js v22.14.0\n";
        assert_eq!(error_line(node).as_deref(), Some("Error: boom"));
        let cmd = "'robocopyy' is not recognized as an internal or external command,\r\noperable program or batch file.\r\n";
        assert_eq!(
            error_line(cmd).as_deref(),
            Some("'robocopyy' is not recognized as an internal or external command, operable program or batch file.")
        );
        assert_eq!(error_line("\n  \n"), None);
    }

    #[test]
    fn last_line_for_silent_scripts() {
        assert_eq!(
            last_line("T", "copying\nDone: 3 files\n\n"),
            "Done: 3 files"
        );
        assert_eq!(last_line("Lock", ""), "Ran Lock.");
    }

    #[test]
    fn short_output_fits_the_footer() {
        assert_eq!(as_notice("Hello, Ana!\n").as_deref(), Some("Hello, Ana!"));
        assert_eq!(
            as_notice("IP: 10.0.0.2\r\n\r\nGateway: 10.0.0.1\r\n").as_deref(),
            Some("IP: 10.0.0.2 · Gateway: 10.0.0.1")
        );
        assert_eq!(as_notice(""), None);
        assert_eq!(as_notice(&"line\n".repeat(60)), None);
    }
}
