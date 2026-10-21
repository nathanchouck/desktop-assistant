//! What tool lines and approval cards say about what Codex does: a command
//! as you would type it, the files a change touches, and why it asks. Pure
//! functions over the items and requests Codex sends.

use std::collections::HashSet;

use serde_json::Value;

use crate::ai::claude::protocol::{file_name, first_line, truncate};
use crate::ai::AiEvent;

/// Characters of a command, diff or tool input shown on an approval card.
pub(super) const DETAIL_MAX: usize = 1200;
pub(super) const SUMMARY_MAX: usize = 80;

pub(super) fn str_field(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

/// The command as you'd type it: Codex's parsed commands (without the
/// `powershell -Command` / `bash -lc` wrapper), else the raw string.
pub(super) fn command_text(p: &Value) -> String {
    let parsed: Vec<&str> = p["commandActions"]
        .as_array()
        .map(|actions| {
            actions
                .iter()
                .filter_map(|a| a["command"].as_str())
                .collect()
        })
        .unwrap_or_default();
    if parsed.is_empty() {
        str_field(p, "command")
    } else {
        parsed.join(" && ")
    }
}

/// A command's tool line: what it does when Codex could tell (read, list,
/// search), else the command itself.
pub(super) fn command_line(item: &Value) -> (String, String) {
    let actions = item["commandActions"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if let [action] = actions {
        match action["type"].as_str() {
            Some("read") => return ("Read".into(), file_name(&str_field(action, "path"))),
            Some("listFiles") => {
                let path = str_field(action, "path");
                if !path.is_empty() {
                    return ("List".into(), file_name(&path));
                }
            }
            Some("search") => {
                let query = str_field(action, "query");
                if !query.is_empty() {
                    return ("Search".into(), query);
                }
            }
            _ => {}
        }
    }
    ("Shell".into(), first_line(&command_text(item)))
}

pub(super) fn command_approval(id: String, p: &Value) -> AiEvent {
    let command = command_text(p);
    let mut detail = command.clone();
    if let Some(cwd) = p["cwd"].as_str().filter(|c| !c.is_empty()) {
        detail.push_str(&format!("\n\nin {cwd}"));
    }
    let (tool, summary) = match p["networkApprovalContext"]["host"].as_str() {
        Some(host) => {
            detail = format!("Connect to {host} for:\n{detail}");
            ("Network".to_owned(), host.to_owned())
        }
        None if p["kind"] == "writeStdin" => ("Shell input".to_owned(), first_line(&command)),
        None => ("Shell".to_owned(), first_line(&command)),
    };
    if let Some(reason) = p["reason"].as_str().filter(|r| !r.trim().is_empty()) {
        detail.push_str(&format!("\nWhy: {}", reason.trim()));
    }
    AiEvent::ApprovalRequested {
        id,
        tool,
        summary: truncate(&summary, SUMMARY_MAX),
        detail: truncate(&detail, DETAIL_MAX),
    }
}

pub(super) fn changes(item: &Value) -> &[Value] {
    item["changes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
}

pub(super) fn change_kind(change: &Value) -> &str {
    let kind = &change["kind"];
    match kind["type"].as_str() {
        Some("update") if kind["move_path"].is_string() => "move",
        Some(other) => other,
        None => "update",
    }
}

/// "Write" (new files), "Delete", "Move", or "Edit".
pub(super) fn change_verb(item: &Value) -> &'static str {
    let kinds: HashSet<&str> = changes(item).iter().map(change_kind).collect();
    match kinds.into_iter().collect::<Vec<_>>().as_slice() {
        ["add"] => "Write",
        ["delete"] => "Delete",
        ["move"] => "Move",
        _ => "Edit",
    }
}

/// "notes.txt", "a.txt, b.txt", "a.txt, b.txt and 3 more".
pub(super) fn changed_files(item: &Value) -> String {
    let names: Vec<String> = changes(item)
        .iter()
        .map(|c| file_name(&str_field(c, "path")))
        .collect();
    match names.len() {
        0..=2 => names.join(", "),
        n => format!("{}, {} and {} more", names[0], names[1], n - 2),
    }
}

pub(super) fn file_approval(id: String, item: &Value, p: &Value) -> AiEvent {
    let mut parts: Vec<String> = changes(item)
        .iter()
        .map(|change| {
            let path = str_field(change, "path");
            let head = match change_kind(change) {
                "move" => format!("{path} → {}", str_field(&change["kind"], "move_path")),
                "delete" => format!("{path} (delete)"),
                _ => path,
            };
            let diff = str_field(change, "diff");
            if diff.is_empty() {
                head
            } else {
                format!("{head}\n\n{}", diff.trim_end())
            }
        })
        .collect();
    if let Some(root) = p["grantRoot"].as_str().filter(|r| !r.is_empty()) {
        parts.insert(0, format!("Write access to {root}"));
    }
    if let Some(reason) = p["reason"].as_str().filter(|r| !r.trim().is_empty()) {
        parts.push(format!("Why: {}", reason.trim()));
    }
    let summary = changed_files(item);
    AiEvent::ApprovalRequested {
        id,
        tool: change_verb(item).into(),
        summary: truncate(&summary, SUMMARY_MAX),
        detail: truncate(&parts.join("\n\n"), DETAIL_MAX),
    }
}

/// `mcp__<server>__<tool>`, the name Claude's tool helpers know.
pub(super) fn qualified(item: &Value) -> String {
    format!(
        "mcp__{}__{}",
        item["server"].as_str().unwrap_or("mcp"),
        item["tool"].as_str().unwrap_or("tool")
    )
}

/// `… run tool "screenshot"?` → `screenshot`.
pub(super) fn quoted_tool(message: &str) -> Option<String> {
    let (_, rest) = message.split_once("tool \"")?;
    let (tool, _) = rest.split_once('"')?;
    (!tool.is_empty()).then(|| tool.to_owned())
}
