import type { ProviderId } from "./bindings";
import { answeringLabel, isThinking, pendingApproval, type Transcript } from "./chat";

/** The search row that asks both AIs (Rust: `ipc::compare::ROW_ID`). */
export const COMPARE_ROW_ID = "ai-compare";

/** Who answers in a comparison, left to right (Ctrl+1, Ctrl+2). */
export const COMPARED: ProviderId[] = ["claude", "chatgpt"];

const COMPARE_PREFIX = /^\s*(both|vs)\s+(?=\S)/i;

/**
 * "both <question>" asks Claude and ChatGPT side by side. So does
 * "vs <question>", but only from three words on, so "vs code" still finds
 * VS Code. The question without the prefix, else null (as Rust's
 * `ai::compare_prefix`).
 */
export function comparePrefix(query: string): string | null {
  const m = COMPARE_PREFIX.exec(query);
  if (!m?.[1]) return null;
  const text = query.slice(m[0].length).trim();
  if (m[1].toLowerCase() === "vs" && text.split(/\s+/).length < 3) return null;
  return text;
}

/** How one side of a comparison is doing. */
export type SideStatus = "thinking" | "answering" | "approval" | "done" | "stopped" | "error";

export const STATUS_LABELS: Record<SideStatus, string> = {
  thinking: "thinking",
  answering: "answering",
  approval: "needs approval",
  done: "done",
  stopped: "stopped",
  error: "error",
};

export function sideStatus(t: Transcript): SideStatus {
  if (t.running) {
    if (pendingApproval(t)) return "approval";
    return isThinking(t) ? "thinking" : "answering";
  }
  const last = t.entries[t.entries.length - 1];
  if (last?.kind === "error") return "error";
  if (last?.kind === "note" && last.text === "Stopped") return "stopped";
  return "done";
}

/**
 * The footer while comparing: who answers on each side and how it's
 * going ("Claude · Sonnet 5.5 · done | ChatGPT · GPT-6.1-Sol · thinking").
 */
export function compareNotice(sides: { provider: ProviderId; transcript: Transcript }[]): string {
  return sides
    .map((s) => `${answeringLabel(s.provider, s.transcript.model)} · ${STATUS_LABELS[sideStatus(s.transcript)]}`)
    .join(" | ");
}
