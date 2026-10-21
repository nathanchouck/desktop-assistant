//! What we send to `codex app-server`, and how we read its replies, as pure
//! functions. JSON-RPC 2.0 without the `"jsonrpc"` field, one JSON object
//! per line. `parser.rs` reads what Codex sends on its own.
//!
//! The protocol is marked experimental and changes between releases, so
//! everything is read leniently from `serde_json::Value`.

use serde_json::{json, Value};

use super::{sandbox, McpServer, Options};
use crate::mcp::protocol::SERVER_NAME;
use crate::mcp::tools::{CONTEXT, SCREENSHOT, SEARCH};

/// Sent as `clientInfo.name`; Codex uses it as the `originator` of requests.
pub const CLIENT_NAME: &str = "desktop_assistant";
/// The release these messages were written and tested against.
pub const MIN_VERSION: (u32, u32, u32) = (0, 163, 0);

/// Notifications we never use, so Codex doesn't send them.
const NOT_NEEDED: &[&str] = &[
    "account/rateLimits/updated",
    "item/commandExecution/outputDelta",
    "item/fileChange/outputDelta",
    "item/reasoning/summaryTextDelta",
    "item/reasoning/textDelta",
    "remoteControl/status/changed",
    "thread/status/changed",
    "thread/tokenUsage/updated",
];

/// The kind of approval Codex is waiting for; each is answered its own way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalKind {
    Command,
    FileChange,
    McpTool,
}

pub fn initialize(id: u64) -> String {
    json!({
        "id": id,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": CLIENT_NAME,
                "title": "Desktop Assistant",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "capabilities": {
                "experimentalApi": false,
                "requestAttestation": false,
                "optOutNotificationMethods": NOT_NEEDED,
            },
        },
    })
    .to_string()
}

pub fn initialized() -> String {
    json!({ "method": "initialized" }).to_string()
}

pub fn get_auth_status(id: u64) -> String {
    json!({
        "id": id,
        "method": "getAuthStatus",
        "params": { "includeToken": false, "refreshToken": false },
    })
    .to_string()
}

/// Signs this process in with an API key (kept in memory only, see
/// `codex::args`). Contains the key: never log this line.
pub fn login_api_key(id: u64, key: &str) -> String {
    json!({
        "id": id,
        "method": "account/login/start",
        "params": { "type": "apiKey", "apiKey": key },
    })
    .to_string()
}

/// The launcher's MCP server for one thread: its address and token go over
/// stdin (never a command line or an environment variable that commands
/// could read). Reading tools need no approval; a screenshot always asks.
pub fn mcp_config(server: &McpServer, screen_tools: bool) -> Value {
    let mut launcher = json!({
        "url": server.url,
        "http_headers": { "Authorization": format!("Bearer {}", server.token) },
        "startup_timeout_sec": 10,
        "tools": {
            CONTEXT: { "approval_mode": "approve" },
            SEARCH: { "approval_mode": "approve" },
            SCREENSHOT: { "approval_mode": "prompt" },
        },
    });
    if !screen_tools {
        launcher["disabled_tools"] = json!([CONTEXT, SCREENSHOT]);
    }
    json!({ "mcp_servers": { SERVER_NAME: launcher } })
}

/// `thread/start`, or `thread/resume` when continuing an earlier thread.
/// Either way approvals go to you ("user", never Codex's auto-reviewer),
/// and the sandbox follows `codex::sandbox`.
pub fn open_thread(id: u64, options: &Options) -> String {
    let mut params = json!({
        "approvalPolicy": "on-request",
        "approvalsReviewer": "user",
        "sandbox": sandbox(options.approvals, &options.cwd, &options.home).as_str(),
        "developerInstructions": options.instructions,
    });
    if !options.model.is_empty() {
        params["model"] = json!(options.model);
    }
    if let Some(server) = &options.mcp {
        params["config"] = mcp_config(server, options.screen_tools);
    }
    let method = match &options.resume {
        Some(thread) => {
            params["threadId"] = json!(thread);
            // Only the thread, not its whole history.
            params["excludeTurns"] = json!(true);
            "thread/resume"
        }
        None => {
            params["cwd"] = json!(options.cwd);
            "thread/start"
        }
    };
    json!({ "id": id, "method": method, "params": params }).to_string()
}

pub fn turn_start(id: u64, thread: &str, text: &str) -> String {
    json!({
        "id": id,
        "method": "turn/start",
        "params": { "threadId": thread, "input": [{ "type": "text", "text": text }] },
    })
    .to_string()
}

pub fn turn_interrupt(id: u64, thread: &str, turn: &str) -> String {
    json!({
        "id": id,
        "method": "turn/interrupt",
        "params": { "threadId": thread, "turnId": turn },
    })
    .to_string()
}

/// Your Approve / Deny, in the shape that kind of request expects. Deny is
/// "decline": the action doesn't happen and ChatGPT carries on.
pub fn approval_response(rpc_id: &Value, kind: ApprovalKind, allow: bool) -> String {
    let result = match kind {
        ApprovalKind::Command | ApprovalKind::FileChange => {
            json!({ "decision": if allow { "accept" } else { "decline" } })
        }
        ApprovalKind::McpTool => json!({
            "action": if allow { "accept" } else { "decline" },
            "content": null,
            "_meta": null,
        }),
    };
    json!({ "id": rpc_id, "result": result }).to_string()
}

/// A reply to a request from Codex.
pub fn reply(rpc_id: &Value, result: Value) -> String {
    json!({ "id": rpc_id, "result": result }).to_string()
}

/// Declines a request from Codex we don't handle.
pub fn error_response(rpc_id: &Value, message: &str) -> String {
    json!({ "id": rpc_id, "error": { "code": -32601, "message": message } }).to_string()
}

/// The CLI version from `initialize`'s `userAgent`
/// (`desktop_assistant/0.163.0 (Windows 11 …) …`).
pub fn version(user_agent: &str) -> Option<(u32, u32, u32)> {
    let version = user_agent.split_once('/')?.1.split_whitespace().next()?;
    let mut parts = version.split(['.', '-']).map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

pub const SIGNED_OUT: &str = "Codex isn't signed in to ChatGPT. Close this chat and pick \
     \"Sign In to ChatGPT\" in the launcher.";
pub const API_KEY_LOGIN: &str = "Codex is signed in with an API key, which bills per token \
     instead of your ChatGPT plan. Pick \"Sign In to ChatGPT\" in the launcher, or set \
     ChatGPT's billing to API key in settings.";

/// What's wrong with `getAuthStatus`'s answer for using the ChatGPT plan,
/// if anything.
pub fn auth_problem(status: &Value) -> Option<&'static str> {
    if status["requiresOpenaiAuth"] == json!(false) {
        // A provider of the user's own that needs no OpenAI sign-in.
        return None;
    }
    match status["authMethod"].as_str() {
        None => Some(SIGNED_OUT),
        Some("apikey") => Some(API_KEY_LOGIN),
        Some(_) => None,
    }
}

/// A failed turn's error, with a hint when there's something to do.
pub fn explain(error: &Value) -> String {
    let message = error["message"]
        .as_str()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or("ChatGPT ran into an error. Try again.");
    let info = &error["codexErrorInfo"];
    let code = info.as_str().or_else(|| {
        info.as_object()
            .and_then(|o| o.keys().next())
            .map(String::as_str)
    });
    let status = info
        .as_object()
        .and_then(|o| o.values().next())
        .and_then(|v| v["httpStatusCode"].as_u64());
    match (code, status) {
        (Some("unauthorized"), _) | (_, Some(401)) => format!(
            "{message} (To sign in, close this chat and pick \"Sign In to ChatGPT\" in the launcher.)"
        ),
        (Some("usageLimitExceeded"), _) => {
            format!("You've reached your ChatGPT plan's usage limit for now. ({message})")
        }
        (Some("rateLimitExceeded"), _) | (_, Some(429)) => {
            "ChatGPT is limiting requests right now. Try again in a moment.".into()
        }
        (Some("contextWindowExceeded"), _) => {
            "This conversation is too long for the model. Press Esc and start a new chat.".into()
        }
        (Some("serverOverloaded" | "internalServerError"), _) => {
            "ChatGPT is overloaded right now. Try again in a moment.".into()
        }
        _ => message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{Approvals, BillingMode};
    use std::path::PathBuf;

    fn options() -> Options {
        Options {
            model: String::new(),
            approvals: Approvals::Ask,
            billing: BillingMode::Subscription,
            api_key: None,
            cwd: PathBuf::from("/home/me/Downloads"),
            home: PathBuf::from("/home/me"),
            resume: None,
            instructions: "be brief".into(),
            mcp: Some(McpServer {
                url: "http://127.0.0.1:4321/mcp".into(),
                token: "secret".into(),
            }),
            screen_tools: true,
            windows_sandbox: false,
        }
    }

    fn parse(line: &str) -> Value {
        serde_json::from_str(line).unwrap()
    }

    #[test]
    fn starts_a_thread_where_you_are() {
        let msg = parse(&open_thread(3, &options()));
        assert_eq!(msg["id"], 3);
        assert_eq!(msg["method"], "thread/start");
        let p = &msg["params"];
        assert_eq!(p["cwd"], "/home/me/Downloads");
        assert_eq!(p["approvalPolicy"], "on-request");
        assert_eq!(p["approvalsReviewer"], "user");
        assert_eq!(p["sandbox"], "read-only");
        assert_eq!(p["developerInstructions"], "be brief");
        assert!(p.get("model").is_none(), "blank model: Codex's default");
        assert!(
            p.get("baseInstructions").is_none(),
            "that would replace Codex's own prompt"
        );
        let launcher = &p["config"]["mcp_servers"]["launcher"];
        assert_eq!(launcher["url"], "http://127.0.0.1:4321/mcp");
        assert_eq!(launcher["http_headers"]["Authorization"], "Bearer secret");
        assert_eq!(launcher["tools"]["screenshot"]["approval_mode"], "prompt");
        assert_eq!(launcher["tools"]["search"]["approval_mode"], "approve");
        assert!(launcher.get("disabled_tools").is_none());
    }

    #[test]
    fn resumes_a_thread_with_todays_settings() {
        let msg = parse(&open_thread(
            2,
            &Options {
                resume: Some("thread-1".into()),
                model: "gpt-6-luna".into(),
                approvals: Approvals::Auto,
                screen_tools: false,
                ..options()
            },
        ));
        assert_eq!(msg["method"], "thread/resume");
        let p = &msg["params"];
        assert_eq!(p["threadId"], "thread-1");
        assert_eq!(p["excludeTurns"], true);
        assert_eq!(p["model"], "gpt-6-luna");
        assert_eq!(p["sandbox"], "workspace-write");
        let launcher = &p["config"]["mcp_servers"]["launcher"];
        assert_eq!(
            launcher["disabled_tools"],
            json!(["get_desktop_context", "screenshot"])
        );
    }

    #[test]
    fn builds_requests() {
        let init = parse(&initialize(1));
        assert_eq!(init["method"], "initialize");
        assert_eq!(init["params"]["clientInfo"]["name"], CLIENT_NAME);
        assert!(init.get("jsonrpc").is_none());
        assert_eq!(parse(&initialized())["method"], "initialized");
        assert!(parse(&initialized()).get("id").is_none());

        let turn = parse(&turn_start(4, "t1", "hi"));
        assert_eq!(turn["params"]["threadId"], "t1");
        assert_eq!(
            turn["params"]["input"][0],
            json!({"type": "text", "text": "hi"})
        );

        let stop = parse(&turn_interrupt(5, "t1", "u1"));
        assert_eq!(stop["method"], "turn/interrupt");
        assert_eq!(stop["params"]["turnId"], "u1");

        let login = parse(&login_api_key(6, "sk-1"));
        assert_eq!(login["params"], json!({"type": "apiKey", "apiKey": "sk-1"}));
    }

    #[test]
    fn answers_each_kind_of_approval() {
        let id = json!(0);
        let yes = parse(&approval_response(&id, ApprovalKind::Command, true));
        assert_eq!(yes, json!({"id": 0, "result": {"decision": "accept"}}));
        let no = parse(&approval_response(&id, ApprovalKind::FileChange, false));
        assert_eq!(no["result"]["decision"], "decline");
        let tool = parse(&approval_response(&json!(7), ApprovalKind::McpTool, true));
        assert_eq!(
            tool,
            json!({"id": 7, "result": {"action": "accept", "content": null, "_meta": null}})
        );
        let err = parse(&error_response(&json!("x"), "no"));
        assert_eq!(err["error"]["code"], -32601);
    }

    #[test]
    fn reads_the_version() {
        assert_eq!(
            version("desktop_assistant/0.163.0 (Windows 11; x86_64) unknown"),
            Some((0, 163, 0))
        );
        assert_eq!(version("x/1.2.3-alpha.1"), Some((1, 2, 3)));
        assert_eq!(version("nonsense"), None);
        assert!(version("x/0.170.2").unwrap() >= MIN_VERSION);
    }

    #[test]
    fn insists_on_the_chatgpt_plan() {
        assert_eq!(
            auth_problem(&json!({"authMethod": "chatgpt", "requiresOpenaiAuth": true})),
            None
        );
        assert_eq!(
            auth_problem(&json!({"authMethod": null, "requiresOpenaiAuth": true})),
            Some(SIGNED_OUT)
        );
        assert_eq!(
            auth_problem(&json!({"authMethod": "apikey"})),
            Some(API_KEY_LOGIN)
        );
        assert_eq!(
            auth_problem(&json!({"authMethod": null, "requiresOpenaiAuth": false})),
            None
        );
    }

    #[test]
    fn explains_errors() {
        let e = json!({"message": "unexpected status 401 Unauthorized", "codexErrorInfo": {"httpConnectionFailed": {"httpStatusCode": 401}}});
        assert!(explain(&e).contains("Sign In to ChatGPT"));
        let e = json!({"message": "x", "codexErrorInfo": "usageLimitExceeded"});
        assert!(explain(&e).contains("usage limit"));
        let e = json!({"message": "x", "codexErrorInfo": "contextWindowExceeded"});
        assert!(explain(&e).contains("new chat"));
        let e = json!({"message": "stream disconnected", "codexErrorInfo": "other"});
        assert_eq!(explain(&e), "stream disconnected");
        assert!(explain(&Value::Null).contains("Try again"));
    }
}
