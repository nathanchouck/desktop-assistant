use super::*;

/// Every line of a recorded session, through one parser.
fn fixture(name: &str) -> Vec<Incoming> {
    let path = format!(
        "{}/tests/fixtures/codex/{name}.jsonl",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut parser = Parser::new("1");
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .flat_map(|line| parser.feed(line, None))
        .collect()
}

fn events(incoming: &[Incoming]) -> Vec<AiEvent> {
    incoming
        .iter()
        .filter_map(|i| match i {
            Incoming::Event(e) | Incoming::Approval { event: e, .. } => Some(e.clone()),
            _ => None,
        })
        .collect()
}

fn kinds(events: &[AiEvent]) -> Vec<&'static str> {
    events
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
        .collect()
}

fn text(events: &[AiEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            AiEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn streams_a_plain_reply() {
    let incoming = fixture("text_reply");
    assert!(incoming.iter().any(|i| matches!(
        i,
        Incoming::Reply {
            id: 3,
            result: Ok(_)
        }
    )));
    assert!(incoming.iter().any(
        |i| matches!(i, Incoming::TurnStarted { turn_id } if turn_id.starts_with("01a15c6b-84d9"))
    ));
    let events = events(&incoming);
    assert_eq!(kinds(&events), ["text", "text", "text", "done"]);
    assert_eq!(text(&events), "Hello from the mock.");
}

#[test]
fn shows_a_message_that_arrives_whole() {
    // After a resume the reply came without deltas.
    let events = events(&fixture("resume"));
    assert_eq!(kinds(&events), ["text", "done"]);
    assert_eq!(text(&events), "Hello from the mock.");
}

#[test]
fn skips_reasoning() {
    let events = events(&fixture("reasoning_reply"));
    assert_eq!(text(&events), "Reasoned answer.");
    assert_eq!(events.last(), Some(&AiEvent::Done));
}

#[test]
fn asks_before_a_command_runs() {
    let incoming = fixture("command_approval_accepted");
    let events = events(&incoming);
    assert_eq!(
        kinds(&events),
        [
            "tool",
            "approval",
            "tool_done",
            "text",
            "text",
            "text",
            "done"
        ]
    );
    assert!(
        matches!(&events[0], AiEvent::ToolStarted { name, summary, .. }
        if name == "Shell" && summary == "printf hi > ran.txt && cat ran.txt")
    );
    let AiEvent::ApprovalRequested {
        id, tool, detail, ..
    } = &events[1]
    else {
        panic!("approval expected");
    };
    assert_eq!(id, "codex-1-0");
    assert_eq!(tool, "Shell");
    assert!(
        detail.starts_with("printf hi > ran.txt") && !detail.contains("bash -lc"),
        "{detail}"
    );
    assert!(detail.contains("in /home/me/work"), "{detail}");
    assert!(incoming.iter().any(|i| matches!(i,
        Incoming::Approval { rpc_id, kind: ApprovalKind::Command, .. } if rpc_id == &json!(0))));
    assert!(incoming
        .iter()
        .any(|i| matches!(i, Incoming::Resolved { rpc_id } if rpc_id == &json!(0))));
    assert!(matches!(&events[2], AiEvent::ToolFinished { ok: true, .. }));
}

#[test]
fn says_why_a_command_wants_out_of_the_sandbox() {
    let events = events(&fixture("escalation_declined"));
    let detail = events
        .iter()
        .find_map(|e| match e {
            AiEvent::ApprovalRequested { detail, .. } => Some(detail.clone()),
            _ => None,
        })
        .expect("approval");
    assert!(detail.contains("Why: Write a file outside the workspace?"));
    assert!(events
        .iter()
        .any(|e| matches!(e, AiEvent::ToolFinished { ok: false, .. })));
}

#[test]
fn shows_the_file_change_it_asks_about() {
    let incoming = fixture("file_change_declined");
    let events = events(&incoming);
    assert_eq!(
        kinds(&events),
        [
            "tool",
            "approval",
            "tool_done",
            "text",
            "text",
            "text",
            "done"
        ]
    );
    assert!(
        matches!(&events[0], AiEvent::ToolStarted { name, summary, .. }
        if name == "Write" && summary == "hello.txt")
    );
    assert!(
        matches!(&events[1], AiEvent::ApprovalRequested { tool, summary, detail, .. }
        if tool == "Write" && summary == "hello.txt"
            && detail.contains("/home/me/work/hello.txt") && detail.contains("hello from codex"))
    );
    assert!(incoming.iter().any(|i| matches!(
        i,
        Incoming::Approval {
            kind: ApprovalKind::FileChange,
            ..
        }
    )));
    assert!(matches!(
        &events[2],
        AiEvent::ToolFinished { ok: false, .. }
    ));

    let accepted = events_of("file_change_accepted");
    assert!(accepted
        .iter()
        .any(|e| matches!(e, AiEvent::ToolFinished { ok: true, .. })));
}

fn events_of(name: &str) -> Vec<AiEvent> {
    events(&fixture(name))
}

#[test]
fn the_screenshot_asks_with_its_own_words() {
    let incoming = fixture("mcp_approval_declined");
    let events = events(&incoming);
    assert_eq!(
        kinds(&events),
        [
            "tool",
            "approval",
            "tool_done",
            "text",
            "text",
            "text",
            "done"
        ]
    );
    assert!(
        matches!(&events[0], AiEvent::ToolStarted { name, summary, .. }
        if name == "launcher · screenshot" && summary == "the window you were in")
    );
    assert!(
        matches!(&events[1], AiEvent::ApprovalRequested { tool, detail, .. }
        if tool == "launcher · screenshot" && detail.contains("ChatGPT gets a picture"))
    );
    assert!(incoming.iter().any(|i| matches!(
        i,
        Incoming::Approval {
            kind: ApprovalKind::McpTool,
            ..
        }
    )));
    assert!(matches!(
        &events[2],
        AiEvent::ToolFinished { ok: false, .. }
    ));
}

#[test]
fn launcher_tools_show_what_they_do() {
    let events = events_of("mcp_tool_call");
    assert!(
        matches!(&events[0], AiEvent::ToolStarted { name, summary, .. }
        if name == "launcher · search" && summary == "notes")
    );
    assert!(matches!(&events[1], AiEvent::ToolFinished { ok: true, .. }));
}

#[test]
fn names_what_a_command_does() {
    let events = events_of("readonly_failing_command");
    assert!(
        matches!(&events[0], AiEvent::ToolStarted { name, summary, .. }
        if name == "Read" && summary == "nonexistent-file")
    );
    assert!(matches!(
        &events[1],
        AiEvent::ToolFinished { ok: false, .. }
    ));
    // `ls` with no folder named: the command itself.
    let events = events_of("readonly_ls_no_approval");
    assert!(
        matches!(&events[0], AiEvent::ToolStarted { name, summary, .. }
        if name == "Shell" && summary == "ls -la")
    );
}

#[test]
fn an_interrupted_turn_reads_as_stopped() {
    let incoming = fixture("interrupted");
    let events = events(&incoming);
    assert_eq!(text(&events), "word0 word1 word2 word3 word4 ");
    assert_eq!(events.last(), Some(&AiEvent::Stopped));
    assert!(incoming.iter().any(|i| matches!(
        i,
        Incoming::Reply {
            id: 4,
            result: Ok(_)
        }
    )));
}

#[test]
fn failed_turns_explain_themselves() {
    let events = events_of("http_401");
    assert!(matches!(events.last(), Some(AiEvent::Error { message })
        if message.contains("Sign In to ChatGPT")));
    assert_eq!(kinds(&events), ["error"], "a retrying error isn't the end");
    let events = events_of("stream_failed");
    assert_eq!(
        events.last(),
        Some(&AiEvent::Error {
            message: "stream disconnected before completion: mock upstream failure".into()
        })
    );
}

fn note(method: &str, params: Value) -> String {
    json!({ "method": method, "params": params }).to_string()
}

#[test]
fn separates_messages_that_follow_each_other() {
    let mut p = Parser::new("1");
    let message = |id: &str| json!({"type": "agentMessage", "id": id, "text": ""});
    let lines = [
        note("turn/started", json!({"turn": {"id": "u1"}})),
        note("item/started", json!({"item": message("m1")})),
        note(
            "item/agentMessage/delta",
            json!({"itemId": "m1", "delta": "One."}),
        ),
        note("item/started", json!({"item": message("m2")})),
        note(
            "item/agentMessage/delta",
            json!({"itemId": "m2", "delta": "Two."}),
        ),
    ];
    let events: Vec<AiEvent> = lines
        .iter()
        .flat_map(|l| events(&p.feed(l, None)))
        .collect();
    assert_eq!(text(&events), "One.\n\nTwo.");
}

#[test]
fn ignores_other_threads() {
    let mut p = Parser::new("1");
    let delta = note(
        "item/agentMessage/delta",
        json!({"threadId": "helper", "itemId": "m", "delta": "x"}),
    );
    assert!(p.feed(&delta, Some("main")).is_empty());
    assert_eq!(p.feed(&delta, None).len(), 1, "until the thread is known");
    let done = note(
        "turn/completed",
        json!({"threadId": "helper", "turn": {"status": "completed"}}),
    );
    assert!(p.feed(&done, Some("main")).is_empty());
}

#[test]
fn answers_requests_it_does_not_show() {
    let mut p = Parser::new("1");
    let request = |method: &str, params: Value| {
        json!({"id": 9, "method": method, "params": params}).to_string()
    };
    let answer = |incoming: Vec<Incoming>| match incoming.as_slice() {
        [Incoming::Answer(line)] => serde_json::from_str::<Value>(line).unwrap(),
        other => panic!("expected an answer, got {other:?}"),
    };
    let unknown = answer(p.feed(&request("item/tool/call", json!({})), None));
    assert_eq!(unknown["id"], 9);
    assert!(unknown["error"]["message"].is_string());
    let permissions = answer(p.feed(
        &request("item/permissions/requestApproval", json!({})),
        None,
    ));
    assert_eq!(
        permissions["result"],
        json!({"permissions": {}, "scope": "turn"})
    );
    let form = answer(p.feed(
        &request(
            "mcpServer/elicitation/request",
            json!({"serverName": "x", "mode": "form", "_meta": null, "message": "Your name?"}),
        ),
        None,
    ));
    assert_eq!(form["result"]["action"], "decline");
}

#[test]
fn approval_ids_differ_between_processes() {
    let a = Parser::new("1").approval_id(&json!(0));
    let b = Parser::new("2").approval_id(&json!(0));
    assert_ne!(a, b);
    assert_eq!(Parser::new("3").approval_id(&json!("r")), "codex-3-r");
}

#[test]
fn an_mcp_approval_without_its_item_reads_the_question() {
    let p = Parser::new("1");
    let event = p.mcp_approval(
        "a".into(),
        &json!({"serverName": "launcher", "message": "Allow the launcher MCP server to run tool \"open\"?",
                "_meta": {"codex_approval_kind": "mcp_tool_call", "tool_params": {"target": "C:\\x.pdf"}}}),
    );
    assert!(
        matches!(event, AiEvent::ApprovalRequested { tool, detail, .. }
        if tool == "launcher · open" && detail == "C:\\x.pdf")
    );
}

#[test]
fn network_and_moves_are_spelled_out() {
    let AiEvent::ApprovalRequested {
        tool,
        summary,
        detail,
        ..
    } = command_approval(
        "a".into(),
        &json!({"command": "curl https://example.com", "networkApprovalContext": {"host": "example.com", "protocol": "https"}}),
    )
    else {
        panic!()
    };
    assert_eq!(
        (tool.as_str(), summary.as_str()),
        ("Network", "example.com")
    );
    assert!(detail.contains("curl https://example.com"));

    let item = json!({"changes": [
        {"path": "/d/a.txt", "kind": {"type": "update", "move_path": "/d/b.txt"}, "diff": ""}]});
    let AiEvent::ApprovalRequested { tool, detail, .. } =
        file_approval("a".into(), &item, &json!({}))
    else {
        panic!()
    };
    assert_eq!(tool, "Move");
    assert!(detail.contains("/d/a.txt → /d/b.txt"));
    let many = json!({"changes": [{"path": "a", "kind": {"type": "add"}}, {"path": "b", "kind": {"type": "delete"}},
                                  {"path": "c", "kind": {"type": "update"}}]});
    assert_eq!(changed_files(&many), "a, b and 1 more");
    assert_eq!(change_verb(&many), "Edit");
}
