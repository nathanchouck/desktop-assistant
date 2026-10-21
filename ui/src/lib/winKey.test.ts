import { describe, expect, it } from "vitest";
import type { WinKeyStatus } from "./bindings";
import { seen, timeoutRow, winKeyStatusText } from "./winKey";

function status(over: Partial<WinKeyStatus> = {}): WinKeyStatus {
  return {
    enabled: true,
    running: true,
    last_seen_secs: 5,
    start_closed: 0,
    late_answers: 0,
    hook_timeout_ms: null,
    hook_timeout_pending: false,
    ...over,
  };
}

describe("seen", () => {
  it("says when the key was last seen", () => {
    expect(seen(null)).toBe("hasn't seen the key yet");
    expect(seen(5)).toBe("saw it 5 s ago");
    expect(seen(150)).toBe("saw it 3 min ago");
    expect(seen(7200)).toBe("saw it 2 h ago");
  });
});

describe("winKeyStatusText", () => {
  it("is empty when the override is off", () => {
    expect(winKeyStatusText(status({ enabled: false }))).toBe("");
  });

  it("says when the hook isn't running", () => {
    expect(winKeyStatusText(status({ running: false }))).toBe("Not running.");
  });

  it("only mentions trouble that happened", () => {
    expect(winKeyStatusText(status())).toBe("Working: saw it 5 s ago.");
  });

  it("counts closed Starts and late answers", () => {
    expect(winKeyStatusText(status({ start_closed: 1, late_answers: 3 }))).toBe(
      "Working: saw it 5 s ago. Closed Start once after a tap. Answered late 3 times.",
    );
  });
});

describe("timeoutRow", () => {
  it("offers a longer timeout while it's Windows' default", () => {
    const row = timeoutRow(status());
    expect(row.longer).toBe(true);
    expect(row.button).toBe("Allow 1 s");
    expect(row.hint).toContain("about 0.3 s");
    expect(row.hint).toContain("sign-in");
  });

  it("names a timeout someone set by hand", () => {
    expect(timeoutRow(status({ hook_timeout_ms: 500 })).hint).toContain("500 ms");
  });

  it("offers to undo once it's a full second", () => {
    const row = timeoutRow(status({ hook_timeout_ms: 1000 }));
    expect(row.longer).toBe(false);
    expect(row.button).toBe("Undo");
    expect(row.hint).not.toContain("sign out");
  });

  it("says a fresh change waits for a sign-out", () => {
    expect(timeoutRow(status({ hook_timeout_ms: 1000, hook_timeout_pending: true })).hint).toContain(
      "after you sign out",
    );
    const undone = timeoutRow(status({ hook_timeout_pending: true }));
    expect(undone.longer).toBe(true);
    expect(undone.hint).toContain("once you sign out");
  });
});
