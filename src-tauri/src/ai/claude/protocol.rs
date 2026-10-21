//! Claude Code's stream-json protocol, as pure functions: what comes out of
//! `claude -p --output-format stream-json --input-format stream-json
//! --include-partial-messages --permission-prompt-tool stdio`, and what we
//! write back. The same protocol the Claude Agent SDK speaks.
//!
//! Recorded real output lives in `tests/fixtures/claude/`.

use serde_json::{json, Value};

use crate::ai::AiEvent;

/// One stdout line, understood.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// Something to show.
    Event(AiEvent),
    /// The session id, from the first `system/init` (used to resume later).
    Session { id: String },
    /// Claude wants to use a tool and is waiting for allow / deny. `input`
    /// must be echoed back on allow.
    Permission {
        request_id: String,
        input: Value,
        event: AiEvent,
    },
    /// Claude no longer needs the answer to this request.
    PermissionWithdrawn { request_id: String },
    /// A control request we don't handle; answer with an error so Claude
    /// doesn't wait on it.
    Unsupported { request_id: String },
}

/// Parses one line of Claude Code output. Unknown or uninteresting lines give
/// nothing, so new message types never break the launcher.
pub fn parse_line(line: &str) -> Vec<Incoming> {
    let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
        return Vec::new();
    };
    // Subagent traffic carries a parent tool id; only the main thread shows.
    if msg.get("parent_tool_use_id").is_some_and(|p| !p.is_null()) {
        return Vec::new();
    }
    match msg["type"].as_str() {
        Some("system") if msg["subtype"] == "init" => {
            let mut out = Vec::new();
            if let Some(id) = msg["session_id"].as_str() {
                out.push(Incoming::Session { id: id.into() });
            }
            out.push(Incoming::Event(AiEvent::Started {
                model: msg["model"].as_str().unwrap_or_default().into(),
            }));
            out
        }
        Some("stream_event") => {
            let event = &msg["event"];
            let delta = &event["delta"];
            if event["type"] == "content_block_delta" && delta["type"] == "text_delta" {
                let text = delta["text"].as_str().unwrap_or_default();
                if !text.is_empty() {
                    return vec![Incoming::Event(AiEvent::TextDelta { text: text.into() })];
                }
            }
            Vec::new()
        }
        Some("assistant") => content(&msg)
            .iter()
            .filter(|block| block["type"] == "tool_use")
            .map(|block| {
                let name = block["name"].as_str().unwrap_or("tool");
                Incoming::Event(AiEvent::ToolStarted {
                    id: block["id"].as_str().unwrap_or_default().into(),
                    name: display_tool_name(name),
                    summary: summarize(name, &block["input"]),
                })
            })
            .collect(),
        Some("user") => content(&msg)
            .iter()
            .filter(|block| block["type"] == "tool_result")
            .map(|block| {
                Incoming::Event(AiEvent::ToolFinished {
                    id: block["tool_use_id"].as_str().unwrap_or_default().into(),
                    ok: !block["is_error"].as_bool().unwrap_or(false),
                })
            })
            .collect(),
        Some("control_request") => {
            let request_id = msg["request_id"].as_str().unwrap_or_default().to_owned();
            let request = &msg["request"];
            if request["subtype"] != "can_use_tool" {
                return vec![Incoming::Unsupported { request_id }];
            }
            let tool = request["tool_name"].as_str().unwrap_or("tool");
            let input = request["input"].clone();
            let event = AiEvent::ApprovalRequested {
                id: request_id.clone(),
                tool: display_tool_name(tool),
                summary: summarize(tool, &input),
                detail: detail(tool, &input),
            };
            vec![Incoming::Permission {
                request_id,
                input,
                event,
            }]
        }
        Some("control_cancel_request") => vec![Incoming::PermissionWithdrawn {
            request_id: msg["request_id"].as_str().unwrap_or_default().into(),
        }],
        Some("result") => {
            let failed = msg["is_error"].as_bool().unwrap_or(false)
                || msg["subtype"].as_str().is_some_and(|s| s != "success");
            if failed {
                let text = msg["result"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .or_else(|| msg["subtype"].as_str())
                    .unwrap_or("unknown error");
                vec![Incoming::Event(AiEvent::Error {
                    message: explain_error(text),
                })]
            } else {
                vec![Incoming::Event(AiEvent::Done)]
            }
        }
        _ => Vec::new(),
    }
}

fn content(msg: &Value) -> &[Value] {
    msg["message"]["content"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// Adds a hint to errors the user can fix.
fn explain_error(text: &str) -> String {
    match text {
        "error_max_turns" => {
            return "Claude stopped after too many steps. Ask it to continue.".into()
        }
        "error_during_execution" => return "Claude Code ran into an error. Try again.".into(),
        _ => {}
    }
    let lower = text.to_lowercase();
    if lower.contains("login") || lower.contains("api key") || lower.contains("authenticat") {
        format!("{text} (Open a terminal, run `claude`, and sign in, then try again.)")
    } else {
        text.to_owned()
    }
}

/// `mcp__desktop__get_selection` → "desktop · get_selection".
fn display_tool_name(tool: &str) -> String {
    match tool
        .strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
    {
        Some((server, name)) => format!("{server} · {name}"),
        None => tool.to_owned(),
    }
}

/// One short line describing a tool call, e.g. the file name or command.
pub fn summarize(tool: &str, input: &Value) -> String {
    let field = |key: &str| input[key].as_str().unwrap_or_default();
    let text = match tool {
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => file_name(field("file_path")),
        "Bash" | "PowerShell" => first_line(field("command")),
        "Glob" | "Grep" => field("pattern").to_owned(),
        "WebFetch" => field("url").to_owned(),
        "WebSearch" => field("query").to_owned(),
        "Task" | "Agent" => field("description").to_owned(),
        "TodoWrite" => "updating the plan".to_owned(),
        _ => String::new(),
    };
    truncate(&text, 80)
}

/// The full picture for an approval card: path and content, the whole
/// command, or the raw input for anything else.
pub fn detail(tool: &str, input: &Value) -> String {
    let field = |key: &str| input[key].as_str().unwrap_or_default();
    let text = match tool {
        "Write" => format!("{}\n\n{}", field("file_path"), field("content")),
        "Edit" | "MultiEdit" => format!(
            "{}\n\n- {}\n+ {}",
            field("file_path"),
            field("old_string"),
            field("new_string")
        ),
        "Bash" | "PowerShell" => field("command").to_owned(),
        _ => serde_json::to_string_pretty(input).unwrap_or_default(),
    };
    truncate(&text, 1200)
}

fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_owned()
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().to_owned()
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max_chars - 1).collect();
    format!("{cut}…")
}

/// A user message line for stdin.
pub fn user_message(text: &str) -> String {
    json!({
        "type": "user",
        "message": { "role": "user", "content": text },
        "parent_tool_use_id": null,
    })
    .to_string()
}

/// The answer to a `can_use_tool` request.
pub fn permission_response(request_id: &str, allow: bool, input: &Value) -> String {
    let decision = if allow {
        json!({ "behavior": "allow", "updatedInput": input })
    } else {
        json!({ "behavior": "deny", "message": "The user declined this action." })
    };
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": request_id, "response": decision },
    })
    .to_string()
}

/// Declines a control request we don't support.
pub fn error_response(request_id: &str, error: &str) -> String {
    json!({
        "type": "control_response",
        "response": { "subtype": "error", "request_id": request_id, "error": error },
    })
    .to_string()
}

/// Stops the current turn (the process stays up for follow-ups).
pub fn interrupt(request_id: &str) -> String {
    json!({
        "type": "control_request",
        "request_id": request_id,
        "request": { "subtype": "interrupt" },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<Incoming> {
        let path = format!(
            "{}/tests/fixtures/claude/{name}.jsonl",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .flat_map(parse_line)
            .collect()
    }

    fn events(incoming: &[Incoming]) -> Vec<AiEvent> {
        incoming
            .iter()
            .filter_map(|i| match i {
                Incoming::Event(e) => Some(e.clone()),
                Incoming::Permission { event, .. } => Some(event.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn streams_a_plain_reply() {
        let incoming = fixture("text_reply");
        assert!(matches!(&incoming[0], Incoming::Session { id } if !id.is_empty()));
        assert_eq!(
            events(&incoming),
            [
                AiEvent::Started {
                    model: "claude-haiku-5-5".into()
                },
                AiEvent::TextDelta {
                    text: "hello from the launcher".into()
                },
                AiEvent::Done,
            ]
        );
    }

    #[test]
    fn surfaces_tools_and_permission_requests() {
        let incoming = fixture("write_with_permission");
        let permission = incoming
            .iter()
            .find_map(|i| match i {
                Incoming::Permission {
                    request_id, input, ..
                } => Some((request_id.clone(), input.clone())),
                _ => None,
            })
            .expect("a permission request");
        assert!(!permission.0.is_empty());
        assert_eq!(permission.1["content"], "launcher test\n");

        let events = events(&incoming);
        let kinds: Vec<&str> = events
            .iter()
            .map(|e| match e {
                AiEvent::Started { .. } => "started",
                AiEvent::TextDelta { .. } => "text",
                AiEvent::ToolStarted { .. } => "tool",
                AiEvent::ToolFinished { .. } => "tool_done",
                AiEvent::ApprovalRequested { .. } => "approval",
                AiEvent::ApprovalResolved { .. } => "resolved",
                AiEvent::Done => "done",
                AiEvent::Stopped => "stopped",
                AiEvent::Error { .. } => "error",
            })
            .collect();
        assert_eq!(
            kinds,
            ["started", "tool", "approval", "tool_done", "text", "done"]
        );
        assert!(
            matches!(&events[1], AiEvent::ToolStarted { name, summary, .. }
            if name == "Write" && summary == "notes.txt")
        );
        assert!(
            matches!(&events[2], AiEvent::ApprovalRequested { detail, .. }
            if detail.contains("/home/me/work/notes.txt") && detail.contains("launcher test"))
        );
        assert!(matches!(&events[3], AiEvent::ToolFinished { ok: true, .. }));
    }

    #[test]
    fn ignores_garbage_and_subagent_lines() {
        assert!(parse_line("not json").is_empty());
        assert!(parse_line(r#"{"type":"something_new"}"#).is_empty());
        let sub = r#"{"type":"stream_event","parent_tool_use_id":"toolu_1","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"x"}}}"#;
        assert!(parse_line(sub).is_empty());
    }

    #[test]
    fn errors_get_a_login_hint() {
        let line = r#"{"type":"result","subtype":"success","is_error":true,"result":"Invalid API key · Please run /login"}"#;
        assert!(matches!(&parse_line(line)[0],
            Incoming::Event(AiEvent::Error { message }) if message.contains("sign in")));
        let line = r#"{"type":"result","subtype":"error_max_turns","is_error":false}"#;
        assert!(matches!(&parse_line(line)[0],
            Incoming::Event(AiEvent::Error { message }) if message.contains("too many steps")));
    }

    #[test]
    fn unknown_control_requests_are_declined() {
        let line =
            r#"{"type":"control_request","request_id":"r1","request":{"subtype":"hook_callback"}}"#;
        assert_eq!(
            parse_line(line),
            [Incoming::Unsupported {
                request_id: "r1".into()
            }]
        );
        let line = r#"{"type":"control_cancel_request","request_id":"r2"}"#;
        assert_eq!(
            parse_line(line),
            [Incoming::PermissionWithdrawn {
                request_id: "r2".into()
            }]
        );
    }

    #[test]
    fn builds_outgoing_lines() {
        let msg: Value = serde_json::from_str(&user_message("hi")).unwrap();
        assert_eq!(msg["type"], "user");
        assert_eq!(msg["message"]["content"], "hi");

        let input = json!({"file_path": "a.txt"});
        let allow: Value = serde_json::from_str(&permission_response("r1", true, &input)).unwrap();
        assert_eq!(allow["response"]["request_id"], "r1");
        assert_eq!(allow["response"]["response"]["behavior"], "allow");
        assert_eq!(allow["response"]["response"]["updatedInput"], input);

        let deny: Value = serde_json::from_str(&permission_response("r1", false, &input)).unwrap();
        assert_eq!(deny["response"]["response"]["behavior"], "deny");

        let stop: Value = serde_json::from_str(&interrupt("r9")).unwrap();
        assert_eq!(stop["request"]["subtype"], "interrupt");
        let err: Value = serde_json::from_str(&error_response("r3", "no")).unwrap();
        assert_eq!(err["response"]["subtype"], "error");
    }

    #[test]
    fn summarizes_tools() {
        assert_eq!(
            summarize("Bash", &json!({"command": "mv a b\necho done"})),
            "mv a b"
        );
        assert_eq!(
            summarize("Read", &json!({"file_path": r"C:\Users\n\notes.md"})),
            "notes.md"
        );
        assert_eq!(
            display_tool_name("mcp__desktop__get_selection"),
            "desktop · get_selection"
        );
        assert_eq!(summarize("Unknown", &json!({})), "");
        assert!(summarize("Bash", &json!({"command": "x".repeat(200)})).ends_with('…'));
    }
}
