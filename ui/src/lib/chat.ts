import type { AiEvent } from "./bindings";

/** One line of the conversation as the chat pane shows it. */
export type Entry =
  /** `context`: what Claude was told about the screen, e.g. "Downloads · 2 selected". */
  | { kind: "user"; text: string; context?: string }
  | { kind: "assistant"; text: string }
  | { kind: "tool"; id: string; name: string; summary: string; state: "running" | "ok" | "failed" }
  | {
      kind: "approval";
      id: string;
      tool: string;
      summary: string;
      detail: string;
      state: "pending" | "allowed" | "denied";
    }
  | { kind: "error"; text: string }
  /** A quiet status line, e.g. "Stopped". */
  | { kind: "note"; text: string };

export interface Transcript {
  entries: Entry[];
  /** A turn is in progress. */
  running: boolean;
  model: string;
}

/** Applies one streamed event to the transcript (in place, returns it). */
export function applyEvent(t: Transcript, event: AiEvent): Transcript {
  switch (event.type) {
    case "started":
      t.model = event.model;
      break;
    case "text_delta": {
      const last = t.entries[t.entries.length - 1];
      if (last?.kind === "assistant") last.text += event.text;
      else t.entries.push({ kind: "assistant", text: event.text });
      break;
    }
    case "tool_started":
      t.entries.push({ kind: "tool", id: event.id, name: event.name, summary: event.summary, state: "running" });
      break;
    case "tool_finished":
      for (const e of t.entries) {
        if (e.kind === "tool" && e.id === event.id) e.state = event.ok ? "ok" : "failed";
      }
      break;
    case "approval_requested":
      t.entries.push({
        kind: "approval",
        id: event.id,
        tool: event.tool,
        summary: event.summary,
        detail: event.detail,
        state: "pending",
      });
      break;
    case "approval_resolved":
      for (const e of t.entries) {
        if (e.kind === "approval" && e.id === event.id) e.state = event.allowed ? "allowed" : "denied";
      }
      break;
    case "done":
      t.running = false;
      break;
    case "stopped":
      t.entries.push({ kind: "note", text: "Stopped" });
      t.running = false;
      break;
    case "error":
      t.entries.push({ kind: "error", text: event.message });
      t.running = false;
      break;
  }
  return t;
}

/** The approval waiting for an answer, if any. */
export function pendingApproval(t: Transcript): Extract<Entry, { kind: "approval" }> | undefined {
  for (let i = t.entries.length - 1; i >= 0; i--) {
    const e = t.entries[i];
    if (e?.kind === "approval" && e.state === "pending") return e;
  }
  return undefined;
}

const CLAUDE_PREFIX = /^\s*c\s+(?=\S)/i;

/** "c what's the weather" goes straight to Claude on Enter. */
export function hasClaudePrefix(query: string): boolean {
  return CLAUDE_PREFIX.test(query);
}

/** "c what's the weather" → "what's the weather". */
export function stripClaudePrefix(query: string): string {
  return query.replace(CLAUDE_PREFIX, "").trim();
}

/** The approval card's title text after the tool name: its summary, unless
 *  the detail below already shows it (a command, a path). */
export function approvalSummary(a: { summary: string; detail: string }): string {
  const shown = a.summary.replace(/…$/, "");
  return shown && !a.detail.includes(shown) ? a.summary : "";
}

/** Show the "thinking" dots: a turn is running and nothing is streaming. */
export function isThinking(t: Transcript): boolean {
  if (!t.running) return false;
  const last = t.entries[t.entries.length - 1];
  if (last?.kind === "assistant") return false;
  if (last?.kind === "approval" && last.state === "pending") return false;
  return true;
}

/** "claude-sonnet-5-5" → "Sonnet 5.5"; anything else as is. */
export function modelLabel(id: string): string {
  const m = /^claude-([a-z]+)-(\d+)(?:-(\d{1,2}))?(?:-\d{8})?$/.exec(id);
  if (!m?.[1] || !m[2]) return id;
  const family = m[1].charAt(0).toUpperCase() + m[1].slice(1);
  return m[3] ? `${family} ${m[2]}.${m[3]}` : `${family} ${m[2]}`;
}

/**
 * Whether the query reads like a question for Claude, so it's worth
 * starting Claude Code early: the `c ` prefix, three or more words, or
 * nothing but the "Ask" row matching.
 */
export function looksLikeAQuestion(query: string, items: { kind: string }[]): boolean {
  const text = query.trim();
  if (hasClaudePrefix(text)) return true;
  if (text.split(/\s+/).length >= 3) return true;
  const onlyAsk = items.length > 0 && items.every((item) => item.kind === "ask_ai");
  return onlyAsk && text.length >= 4;
}
