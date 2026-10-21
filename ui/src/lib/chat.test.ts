import { describe, expect, it } from "vitest";
import { applyEvent, approvalSummary, hasClaudePrefix, isThinking, modelLabel, pendingApproval, stripClaudePrefix, type Transcript } from "./chat";

const fresh = (): Transcript => ({ entries: [{ kind: "user", text: "hi" }], running: true, model: "" });

describe("applyEvent", () => {
  it("streams text into one reply", () => {
    const t = fresh();
    applyEvent(t, { type: "started", model: "claude-sonnet-5-5" });
    applyEvent(t, { type: "text_delta", text: "Hel" });
    applyEvent(t, { type: "text_delta", text: "lo" });
    applyEvent(t, { type: "done" });
    expect(t.model).toBe("claude-sonnet-5-5");
    expect(t.entries).toEqual([
      { kind: "user", text: "hi" },
      { kind: "assistant", text: "Hello" },
    ]);
    expect(t.running).toBe(false);
  });

  it("tracks tools and approvals in order", () => {
    const t = fresh();
    applyEvent(t, { type: "tool_started", id: "t1", name: "Write", summary: "notes.txt" });
    applyEvent(t, { type: "approval_requested", id: "r1", tool: "Write", summary: "notes.txt", detail: "x" });
    expect(pendingApproval(t)?.id).toBe("r1");
    applyEvent(t, { type: "approval_resolved", id: "r1", allowed: true });
    applyEvent(t, { type: "tool_finished", id: "t1", ok: true });
    applyEvent(t, { type: "text_delta", text: "done" });
    expect(pendingApproval(t)).toBeUndefined();
    expect(t.entries.map((e) => e.kind)).toEqual(["user", "tool", "approval", "assistant"]);
    expect(t.entries[1]).toMatchObject({ state: "ok" });
    expect(t.entries[2]).toMatchObject({ state: "allowed" });
  });

  it("text after a tool starts a new reply", () => {
    const t = fresh();
    applyEvent(t, { type: "text_delta", text: "Looking…" });
    applyEvent(t, { type: "tool_started", id: "t1", name: "Glob", summary: "*.pdf" });
    applyEvent(t, { type: "text_delta", text: "Found 3." });
    expect(t.entries.filter((e) => e.kind === "assistant")).toHaveLength(2);
  });

  it("errors end the turn", () => {
    const t = fresh();
    applyEvent(t, { type: "error", message: "not signed in" });
    expect(t.running).toBe(false);
    expect(t.entries[1]).toEqual({ kind: "error", text: "not signed in" });
  });
});

describe("stripClaudePrefix", () => {
  it("removes a leading c and space", () => {
    expect(stripClaudePrefix("c what is 2+2")).toBe("what is 2+2");
    expect(stripClaudePrefix("C  sort my downloads")).toBe("sort my downloads");
    expect(stripClaudePrefix("calculator")).toBe("calculator");
  });

  it("only counts a prefix followed by a question", () => {
    expect(hasClaudePrefix("c sort my downloads")).toBe(true);
    expect(hasClaudePrefix("c ")).toBe(false);
    expect(hasClaudePrefix("calc")).toBe(false);
  });
});

describe("stopping", () => {
  it("ends the turn with a quiet note", () => {
    const t = fresh();
    applyEvent(t, { type: "text_delta", text: "Once upon" });
    applyEvent(t, { type: "stopped" });
    expect(t.running).toBe(false);
    expect(t.entries.at(-1)).toEqual({ kind: "note", text: "Stopped" });
  });
});

describe("isThinking", () => {
  it("shows while waiting, hides while text streams or an approval waits", () => {
    const t = fresh();
    expect(isThinking(t)).toBe(true);
    applyEvent(t, { type: "text_delta", text: "Sure" });
    expect(isThinking(t)).toBe(false);
    applyEvent(t, { type: "approval_requested", id: "r1", tool: "Bash", summary: "mv", detail: "mv a b" });
    expect(isThinking(t)).toBe(false);
    applyEvent(t, { type: "approval_resolved", id: "r1", allowed: true });
    expect(isThinking(t)).toBe(true);
    applyEvent(t, { type: "done" });
    expect(isThinking(t)).toBe(false);
  });
});

describe("modelLabel", () => {
  it("names Claude models", () => {
    expect(modelLabel("claude-sonnet-5-5")).toBe("Sonnet 5.5");
    expect(modelLabel("claude-haiku-5-5")).toBe("Haiku 5.5");
    expect(modelLabel("claude-opus-4-20250514")).toBe("Opus 4");
    expect(modelLabel("gpt-5")).toBe("gpt-5");
  });
});

describe("approvalSummary", () => {
  it("skips a summary the detail already shows, even cut short", () => {
    expect(approvalSummary({ summary: "rm ~/Downloads/a very long…", detail: "rm ~/Downloads/a very long name" })).toBe("");
    expect(approvalSummary({ summary: "notes.txt", detail: "/home/me/notes.txt\n\nhello" })).toBe("");
    expect(approvalSummary({ summary: "inbox", detail: '{"label": "x"}' })).toBe("inbox");
    expect(approvalSummary({ summary: "", detail: "{}" })).toBe("");
  });
});
