import type { WinKeyStatus } from "./bindings";

/** What "give the hook more time" sets, in ms (the most Windows allows). */
export const LONGER_TIMEOUT_MS = 1000;

/** "saw it 5 s ago", for the Windows key status line. */
export function seen(secs: number | null): string {
  if (secs === null) return "hasn't seen the key yet";
  if (secs < 60) return `saw it ${secs} s ago`;
  if (secs < 3600) return `saw it ${Math.round(secs / 60)} min ago`;
  return `saw it ${Math.round(secs / 3600)} h ago`;
}

function times(n: number): string {
  return n === 1 ? "once" : `${n} times`;
}

/** The status after "Opens the launcher instead of Start." (empty when off). */
export function winKeyStatusText(status: WinKeyStatus): string {
  if (!status.enabled) return "";
  if (!status.running) return "Not running.";
  const parts = [`Working: ${seen(status.last_seen_secs)}.`];
  if (status.start_closed > 0) parts.push(`Closed Start ${times(status.start_closed)} after a tap.`);
  if (status.late_answers > 0) parts.push(`Answered late ${times(status.late_answers)}.`);
  return parts.join(" ");
}

/** The "more time" row: what it says, and what its button does. */
export interface TimeoutRow {
  hint: string;
  button: string;
  /** What the button asks for: a longer timeout, or Windows' default back. */
  longer: boolean;
}

export function timeoutRow(status: WinKeyStatus): TimeoutRow {
  const ms = status.hook_timeout_ms;
  const pending = status.hook_timeout_pending;
  if (ms !== null && ms >= LONGER_TIMEOUT_MS) {
    const after = pending ? " Applies after you sign out and back in." : "";
    return {
      hint: `Windows waits up to 1 s for the launcher before using a tap itself.${after}`,
      button: "Undo",
      longer: false,
    };
  }
  const wait = ms === null ? "about 0.3 s" : `${ms} ms`;
  const hint = pending
    ? `Windows waits ${wait} again once you sign out and back in.`
    : `If the launcher takes longer than ${wait} to answer a tap, Windows uses it itself ` +
      "and opens Start. Allowing 1 s makes that rarer, from your next sign-in.";
  return { hint, button: "Allow 1 s", longer: true };
}
