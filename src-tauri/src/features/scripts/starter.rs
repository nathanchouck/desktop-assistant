//! The scripts folder as it's first made: a short README and one example
//! command to copy from.

use std::path::Path;

pub const README: &str = "\
Script commands
===============

Every script in this folder that starts with a header comment is a
launcher command: type its title in the launcher and press Enter.

Supported: .ps1 (Windows PowerShell), .py (Python: py -3, else python),
.cmd and .bat, .js (Node.js), .sh (Git Bash, when it's installed).

The header is the comment lines at the very top of the file: # in
PowerShell, Python and Bash, :: or REM in batch files, // in JavaScript.

  # @title Say Hello           required: the command's name
  # @description Greets you    shown under the title
  # @icon 👋                   an emoji, or a launcher symbol (e.g. script)
  # @mode output               what happens to what it prints (below)
  # @argument name             takes text: type \"say hello Ana\"
  # @keywords greet, hi        more words that find it
  # @cwd explorer              run in File Explorer's folder

@mode
  silent  run in the background, then show the last line it printed
          (the default)
  output  show what it printed: in the launcher when it's about a line,
          else in a text file
  copy    copy what it printed
  paste   paste what it printed into the window you came from

With @argument, the text typed after the title is the script's first
argument: $args[0] or param() in PowerShell, sys.argv[1] in Python,
%~1 in batch files, process.argv[2] in Node.js, $1 in Bash.

Scripts run without a window, in this folder (or, with @cwd explorer,
in the folder File Explorer showed when you opened the launcher). They
are stopped, with anything they started, after a time limit: 60 seconds
unless you change it in Settings > Script commands. Exit with a non-zero
code (exit 1) to report a failure; the launcher then shows the last
error line.

Changes here are picked up at once. Subfolders are ignored, so helper
files can go in one.

Scripts run with your own rights, like anything you start yourself. Only
put scripts here that you wrote or trust.

Windows PowerShell reads a .ps1 file without a byte order mark as ANSI:
save scripts as \"UTF-8 with BOM\" if they contain accented letters or
emoji. To print emoji, also start the script with
  [Console]::OutputEncoding = [Text.Encoding]::UTF8
";

pub const HELLO: &str = "\
# @title Say Hello
# @description An example script command: edit it or delete it
# @icon 👋
# @mode output
# @argument name
# @keywords example, greet
#
# The comments above make this file a launcher command. Type \"say hello\"
# (or \"say hello Ana\") in the launcher and press Enter. README.txt in this
# folder lists every option.

param([string]$Name = \"there\")

\"Hello, $Name! It's $(Get-Date -Format t) on $env:COMPUTERNAME.\"
";

/// Text files are written with a UTF-8 byte order mark, so Windows
/// PowerShell 5.1 and older Notepads read the emoji right.
const BOM: &str = "\u{FEFF}";

/// Makes `folder` with the README and the example. Only for a folder that
/// doesn't exist yet, so a deleted example stays deleted.
pub fn create(folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    std::fs::write(folder.join("README.txt"), windows_lines(README))?;
    std::fs::write(folder.join("hello.ps1"), windows_lines(HELLO))?;
    Ok(())
}

fn windows_lines(text: &str) -> String {
    format!("{BOM}{}", text.replace('\n', "\r\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::scripts::header::{self, CommentStyle, ScriptMode};

    #[test]
    fn the_example_is_a_working_command() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("scripts");
        create(&folder).unwrap();
        let bytes = std::fs::read(folder.join("hello.ps1")).unwrap();
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        let h = header::parse(&header::decode(&bytes), CommentStyle::Hash).unwrap();
        assert_eq!(h.title, "Say Hello");
        assert_eq!(h.mode, ScriptMode::Output);
        assert_eq!(h.argument.as_deref(), Some("name"));
        assert_eq!(h.icon.as_deref(), Some("👋"));
        let readme = std::fs::read_to_string(folder.join("README.txt")).unwrap();
        assert!(readme.contains("@title") && readme.contains("\r\n"));
    }
}
