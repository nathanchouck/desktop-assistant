//! MCP's JSON-RPC messages: initialize, tools/list, tools/call, ping. Pure
//! and transport-free, so it's tested without a server.

use async_trait::async_trait;
use serde_json::{json, Value};

/// The server's name as Claude sees it: tools show up as
/// `mcp__launcher__<tool>`.
pub const SERVER_NAME: &str = "launcher";
/// Answered when a client asks for a version we don't know.
const LATEST_VERSION: &str = "2025-06-18";
const KNOWN_VERSIONS: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

/// A tool as `tools/list` describes it.
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema for the arguments.
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Text(String),
    /// A PNG, base64-encoded.
    Png(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub content: Vec<Content>,
    /// The tool ran but failed; Claude sees the text as the reason.
    pub is_error: bool,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(text.into())],
            is_error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(text.into())],
            is_error: true,
        }
    }
}

/// What the server offers.
#[async_trait]
pub trait ToolHost: Send + Sync {
    fn tools(&self) -> Vec<ToolInfo>;
    /// `name` is one of `tools()`.
    async fn call(&self, name: &str, args: Value) -> ToolOutput;
}

/// Answers one JSON-RPC message. `None` for notifications (no reply).
pub async fn handle(host: &dyn ToolHost, message: &Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message["method"].as_str().unwrap_or_default();
    // Notifications (no id) and responses to us need no answer.
    let id = id?;
    let result = match method {
        "initialize" => Ok(initialize(&message["params"])),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools_list(host)),
        "tools/call" => tools_call(host, &message["params"]).await,
        "" => Err((-32600, "not a request".to_owned())),
        other => Err((-32601, format!("unknown method {other}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        }),
    })
}

fn initialize(params: &Value) -> Value {
    let asked = params["protocolVersion"].as_str().unwrap_or(LATEST_VERSION);
    let version = if KNOWN_VERSIONS.contains(&asked) {
        asked
    } else {
        LATEST_VERSION
    };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Tools from the desktop launcher the user is talking to you through: \
            what's on their screen, a fast search of their apps and files, opening and \
            revealing files, and a screenshot of the window they were using.",
    })
}

fn tools_list(host: &dyn ToolHost) -> Value {
    let tools: Vec<Value> = host
        .tools()
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })
        })
        .collect();
    json!({ "tools": tools })
}

async fn tools_call(host: &dyn ToolHost, params: &Value) -> Result<Value, (i64, String)> {
    let name = params["name"].as_str().unwrap_or_default();
    if !host.tools().iter().any(|t| t.name == name) {
        return Err((-32602, format!("unknown tool {name}")));
    }
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let output = host.call(name, args).await;
    let content: Vec<Value> = output
        .content
        .into_iter()
        .map(|c| match c {
            Content::Text(text) => json!({ "type": "text", "text": text }),
            Content::Png(data) => json!({ "type": "image", "data": data, "mimeType": "image/png" }),
        })
        .collect();
    Ok(json!({ "content": content, "isError": output.is_error }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One tool, `echo`, that returns its `text` argument.
    pub(crate) struct Echo;

    #[async_trait]
    impl ToolHost for Echo {
        fn tools(&self) -> Vec<ToolInfo> {
            vec![ToolInfo {
                name: "echo",
                description: "Says it back",
                input_schema: json!({
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"],
                }),
            }]
        }

        async fn call(&self, _name: &str, args: Value) -> ToolOutput {
            match args["text"].as_str() {
                Some(text) => ToolOutput::text(text),
                None => ToolOutput::error("text is required"),
            }
        }
    }

    #[tokio::test]
    async fn initializes_with_a_known_version() {
        let reply = handle(
            &Echo,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {"protocolVersion": "2025-03-26", "capabilities": {}}}),
        )
        .await
        .unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(reply["result"]["serverInfo"]["name"], "launcher");
        assert!(reply["result"]["capabilities"]["tools"].is_object());

        let future = handle(
            &Echo,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "initialize",
                    "params": {"protocolVersion": "2099-01-01"}}),
        )
        .await
        .unwrap();
        assert_eq!(future["result"]["protocolVersion"], LATEST_VERSION);
    }

    #[tokio::test]
    async fn lists_and_calls_tools() {
        let list = handle(
            &Echo,
            &json!({"jsonrpc": "2.0", "id": "a", "method": "tools/list"}),
        )
        .await
        .unwrap();
        assert_eq!(list["id"], "a");
        assert_eq!(list["result"]["tools"][0]["name"], "echo");
        assert_eq!(
            list["result"]["tools"][0]["inputSchema"]["required"][0],
            "text"
        );

        let call = handle(
            &Echo,
            &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": {"name": "echo", "arguments": {"text": "hi"}}}),
        )
        .await
        .unwrap();
        assert_eq!(
            call["result"]["content"][0],
            json!({"type": "text", "text": "hi"})
        );
        assert_eq!(call["result"]["isError"], false);

        let failed = handle(
            &Echo,
            &json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "echo"}}),
        )
        .await
        .unwrap();
        assert_eq!(failed["result"]["isError"], true);
    }

    #[tokio::test]
    async fn rejects_unknown_tools_and_methods() {
        let unknown_tool = handle(
            &Echo,
            &json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "rm"}}),
        )
        .await
        .unwrap();
        assert_eq!(unknown_tool["error"]["code"], -32602);
        let unknown_method = handle(
            &Echo,
            &json!({"jsonrpc": "2.0", "id": 6, "method": "resources/list"}),
        )
        .await
        .unwrap();
        assert_eq!(unknown_method["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn notifications_get_no_reply() {
        let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert!(handle(&Echo, &note).await.is_none());
    }
}
