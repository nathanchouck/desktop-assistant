//! What `codex app-server` sends us, turned into `AiEvent`s: streamed
//! text, tool lines, approval requests and the end of each turn. Pure apart
//! from a little memory within a turn (an approval request refers back to
//! the item that started just before it).
//!
//! Recorded output lives in `tests/fixtures/codex/`.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use super::describe::{
    change_verb, changed_files, command_approval, command_line, file_approval, qualified,
    quoted_tool, str_field, SUMMARY_MAX,
};
use super::protocol::{error_response, explain, reply, ApprovalKind};
use crate::ai::claude::protocol::{
    detail as tool_detail, display_tool_name, file_name, summarize, truncate,
};
use crate::ai::AiEvent;

/// One message from Codex, understood.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// Something to show.
    Event(AiEvent),
    /// The answer to one of our requests.
    Reply {
        id: u64,
        result: Result<Value, String>,
    },
    /// A turn began; interrupting it needs its id.
    TurnStarted { turn_id: String },
    /// Codex waits for Approve / Deny; `rpc_id` goes back with the answer.
    Approval {
        rpc_id: Value,
        kind: ApprovalKind,
        event: AiEvent,
    },
    /// Codex stopped waiting for the answer to `rpc_id` (or got it).
    Resolved { rpc_id: Value },
    /// A request we answer on the spot (never shown): send this line back.
    Answer(String),
}

/// Reads Codex's messages for one process.
#[derive(Debug, Default)]
pub struct Parser {
    /// Makes approval ids unique across processes of one conversation
    /// (Codex numbers its requests from 0 in each process).
    prefix: String,
    /// File changes started this turn, by item id: their approval request
    /// only names the item.
    file_changes: HashMap<String, Value>,
    /// The MCP tool call that started last: its approval request doesn't
    /// say which tool it is.
    last_mcp: Option<Value>,
    /// Messages that streamed text, so their final copy isn't shown twice.
    streamed: HashSet<String>,
    /// The last thing shown was answer text.
    after_text: bool,
    /// A new message started after text: put a paragraph break first.
    needs_break: bool,
}

impl Parser {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            ..Self::default()
        }
    }

    /// The id the UI uses for approval request `rpc_id`.
    pub fn approval_id(&self, rpc_id: &Value) -> String {
        let id = match rpc_id {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        format!("codex-{}-{id}", self.prefix)
    }

    /// Parses one stdout line. `main_thread` is the conversation's thread
    /// once known: notifications about other threads (Codex's helper
    /// agents) aren't shown. Unknown messages give nothing.
    pub fn feed(&mut self, line: &str, main_thread: Option<&str>) -> Vec<Incoming> {
        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        let has_id = msg.get("id").is_some_and(|id| !id.is_null());
        match (msg["method"].as_str(), has_id) {
            (None, true) => reply_to_us(&msg).into_iter().collect(),
            (Some(method), true) => vec![self.request(method, &msg)],
            (Some(method), false) => {
                let params = &msg["params"];
                let other_thread = matches!(
                    (params["threadId"].as_str(), main_thread),
                    (Some(thread), Some(main)) if thread != main
                );
                if other_thread && method != "serverRequest/resolved" {
                    return Vec::new();
                }
                self.notification(method, params)
            }
            (None, false) => Vec::new(),
        }
    }

    /// A request from Codex that needs an answer.
    fn request(&mut self, method: &str, msg: &Value) -> Incoming {
        let rpc_id = msg["id"].clone();
        let p = &msg["params"];
        let id = self.approval_id(&rpc_id);
        let (kind, event) = match method {
            "item/commandExecution/requestApproval" => {
                (ApprovalKind::Command, command_approval(id, p))
            }
            "item/fileChange/requestApproval" => {
                let item = p["itemId"]
                    .as_str()
                    .and_then(|i| self.file_changes.get(i))
                    .cloned()
                    .unwrap_or(Value::Null);
                (ApprovalKind::FileChange, file_approval(id, &item, p))
            }
            "mcpServer/elicitation/request"
                if p["_meta"]["codex_approval_kind"] == "mcp_tool_call" =>
            {
                (ApprovalKind::McpTool, self.mcp_approval(id, p))
            }
            // A real form from an MCP server: not something the launcher shows.
            "mcpServer/elicitation/request" => {
                return Incoming::Answer(reply(
                    &rpc_id,
                    json!({ "action": "decline", "content": null, "_meta": null }),
                ))
            }
            // Extra sandbox permissions: grant nothing; the command can
            // still ask to run outside the sandbox, which you approve.
            "item/permissions/requestApproval" => {
                return Incoming::Answer(reply(
                    &rpc_id,
                    json!({ "permissions": {}, "scope": "turn" }),
                ))
            }
            "item/tool/requestUserInput" => {
                return Incoming::Answer(reply(&rpc_id, json!({ "answers": {} })))
            }
            _ => return Incoming::Answer(error_response(&rpc_id, "not supported by the launcher")),
        };
        Incoming::Approval {
            rpc_id,
            kind,
            event,
        }
    }

    fn notification(&mut self, method: &str, p: &Value) -> Vec<Incoming> {
        let item = &p["item"];
        match method {
            "turn/started" => {
                self.after_text = false;
                self.needs_break = false;
                p["turn"]["id"]
                    .as_str()
                    .map(|id| Incoming::TurnStarted { turn_id: id.into() })
                    .into_iter()
                    .collect()
            }
            "item/agentMessage/delta" => {
                let delta = p["delta"].as_str().unwrap_or_default();
                if delta.is_empty() {
                    return Vec::new();
                }
                if let Some(item_id) = p["itemId"].as_str() {
                    self.streamed.insert(item_id.to_owned());
                }
                vec![self.text(delta)]
            }
            "item/started" if item["type"] == "agentMessage" => {
                self.needs_break = self.after_text;
                Vec::new()
            }
            // A message that arrived whole, without deltas.
            "item/completed" if item["type"] == "agentMessage" => {
                let text = item["text"].as_str().unwrap_or_default();
                let streamed = item["id"]
                    .as_str()
                    .is_some_and(|id| self.streamed.contains(id));
                if streamed || text.is_empty() {
                    return Vec::new();
                }
                vec![self.text(text)]
            }
            "item/started" => self.tool_started(item).into_iter().collect(),
            "item/completed" => self.tool_finished(item).into_iter().collect(),
            "item/fileChange/patchUpdated" => {
                if let Some(cached) = p["itemId"]
                    .as_str()
                    .and_then(|id| self.file_changes.get_mut(id))
                {
                    cached["changes"] = p["changes"].clone();
                }
                Vec::new()
            }
            // The model behind the answer changed (e.g. a fallback).
            "model/rerouted" => p["toModel"]
                .as_str()
                .map(|model| {
                    Incoming::Event(AiEvent::Started {
                        model: model.into(),
                    })
                })
                .into_iter()
                .collect(),
            "serverRequest/resolved" => vec![Incoming::Resolved {
                rpc_id: p["requestId"].clone(),
            }],
            "turn/completed" => {
                let turn = &p["turn"];
                let event = match turn["status"].as_str() {
                    Some("interrupted") => AiEvent::Stopped,
                    Some("failed") => AiEvent::Error {
                        message: explain(&turn["error"]),
                    },
                    _ if !turn["error"].is_null() => AiEvent::Error {
                        message: explain(&turn["error"]),
                    },
                    _ => AiEvent::Done,
                };
                self.file_changes.clear();
                self.streamed.clear();
                self.last_mcp = None;
                self.after_text = false;
                self.needs_break = false;
                vec![Incoming::Event(event)]
            }
            _ => Vec::new(),
        }
    }

    fn text(&mut self, text: &str) -> Incoming {
        let text = if std::mem::take(&mut self.needs_break) {
            format!("\n\n{text}")
        } else {
            text.to_owned()
        };
        self.after_text = true;
        Incoming::Event(AiEvent::TextDelta { text })
    }

    fn tool_started(&mut self, item: &Value) -> Option<Incoming> {
        let id = item["id"].as_str()?.to_owned();
        let (name, summary) = match item["type"].as_str()? {
            "commandExecution" => command_line(item),
            "fileChange" => {
                self.file_changes.insert(id.clone(), item.clone());
                (change_verb(item).to_owned(), changed_files(item))
            }
            "mcpToolCall" => {
                self.last_mcp = Some(item.clone());
                let name = qualified(item);
                (
                    display_tool_name(&name),
                    summarize(&name, &item["arguments"]),
                )
            }
            "webSearch" => ("Web search".into(), str_field(item, "query")),
            "imageView" => ("View image".into(), file_name(&str_field(item, "path"))),
            _ => return None,
        };
        self.after_text = false;
        Some(Incoming::Event(AiEvent::ToolStarted {
            id,
            name,
            summary: truncate(&summary, SUMMARY_MAX),
        }))
    }

    fn tool_finished(&mut self, item: &Value) -> Option<Incoming> {
        let id = item["id"].as_str()?.to_owned();
        let status = item["status"].as_str();
        let ok = match item["type"].as_str()? {
            "commandExecution" => {
                status == Some("completed") && item["exitCode"].as_i64() == Some(0)
            }
            "fileChange" => {
                self.file_changes.remove(&id);
                status == Some("completed")
            }
            "mcpToolCall" => status == Some("completed"),
            "webSearch" | "imageView" => true,
            _ => return None,
        };
        Some(Incoming::Event(AiEvent::ToolFinished { id, ok }))
    }

    fn mcp_approval(&self, id: String, p: &Value) -> AiEvent {
        let server = p["serverName"].as_str().unwrap_or("mcp");
        // The request doesn't name the tool: it's the call that started
        // just before, else the one in the question
        // (`Allow the launcher MCP server to run tool "screenshot"?`).
        let started = self
            .last_mcp
            .as_ref()
            .filter(|item| item["server"].as_str() == Some(server));
        let tool = started
            .and_then(|item| item["tool"].as_str())
            .map(str::to_owned)
            .or_else(|| quoted_tool(p["message"].as_str().unwrap_or_default()))
            .unwrap_or_else(|| "tool".into());
        let args = started
            .map(|item| item["arguments"].clone())
            .unwrap_or_else(|| p["_meta"]["tool_params"].clone());
        let name = format!("mcp__{server}__{tool}");
        let detail = if name == crate::mcp::tools::qualified(crate::mcp::tools::SCREENSHOT) {
            "ChatGPT gets a picture of the window you were in (never the launcher).".to_owned()
        } else {
            tool_detail(&name, &args)
        };
        AiEvent::ApprovalRequested {
            id,
            tool: display_tool_name(&name),
            summary: summarize(&name, &args),
            detail,
        }
    }
}

/// Our request `id` was answered.
fn reply_to_us(msg: &Value) -> Option<Incoming> {
    let id = msg["id"].as_u64()?;
    let result = match msg.get("error") {
        Some(error) => Err(error["message"]
            .as_str()
            .unwrap_or("Codex reported an error")
            .to_owned()),
        None => Ok(msg["result"].clone()),
    };
    Some(Incoming::Reply { id, result })
}

#[cfg(test)]
mod tests;
