import { describe, expect, it } from "vitest";
import type { UpdatePhase, UpdateStatus } from "./bindings";
import { canCheck, checkedAgo, isNewer, readyVersion, statusLine } from "./updates";

const NOW = 1_800_000_000_000; // ms
const SECONDS = NOW / 1000;

const status = (phase: UpdatePhase, summary: string, last_checked: number | null = null, revision = 1): UpdateStatus => ({
  current_version: "0.1.0",
  phase,
  summary,
  last_checked,
  revision,
});

describe("canCheck / readyVersion", () => {
  it("checks only when set up and idle", () => {
    expect(canCheck({ state: "idle" })).toBe(true);
    expect(canCheck({ state: "up_to_date" })).toBe(true);
    expect(canCheck({ state: "failed", message: "offline" })).toBe(true);
    expect(canCheck({ state: "ready", version: "0.2.0" })).toBe(true);
    expect(canCheck({ state: "not_set_up" })).toBe(false);
    expect(canCheck({ state: "checking" })).toBe(false);
    expect(canCheck({ state: "downloading", version: "0.2.0", percent: 40 })).toBe(false);
    expect(canCheck({ state: "installing", version: "0.2.0" })).toBe(false);
  });

  it("offers install only when downloaded", () => {
    expect(readyVersion({ state: "ready", version: "0.2.0" })).toBe("0.2.0");
    expect(readyVersion({ state: "downloading", version: "0.2.0", percent: null })).toBeNull();
    expect(readyVersion({ state: "installing", version: "0.2.0" })).toBeNull();
  });
});

describe("checkedAgo", () => {
  it("says how long ago in words", () => {
    expect(checkedAgo(null, NOW)).toBe("not checked on this PC yet");
    expect(checkedAgo(SECONDS - 20, NOW)).toBe("checked just now");
    expect(checkedAgo(SECONDS - 60, NOW)).toBe("checked 1 minute ago");
    expect(checkedAgo(SECONDS - 59 * 60, NOW)).toBe("checked 59 minutes ago");
    expect(checkedAgo(SECONDS - 3600, NOW)).toBe("checked 1 hour ago");
    expect(checkedAgo(SECONDS - 6 * 3600, NOW)).toBe("checked 6 hours ago");
    expect(checkedAgo(SECONDS - 30 * 3600, NOW)).toBe("checked yesterday");
    expect(checkedAgo(SECONDS - 5 * 86400, NOW)).toBe("checked 5 days ago");
  });

  it("treats a time in the future as just now", () => {
    expect(checkedAgo(SECONDS + 3600, NOW)).toBe("checked just now");
  });
});

describe("statusLine", () => {
  it("adds when it last checked to idle and up to date", () => {
    expect(statusLine(status({ state: "idle" }, "Not checked yet"), NOW)).toBe("Not checked yet");
    expect(statusLine(status({ state: "idle" }, "Not checked yet", SECONDS - 7200), NOW)).toBe("Checked 2 hours ago");
    expect(statusLine(status({ state: "up_to_date" }, "Up to date", SECONDS), NOW)).toBe("Up to date · checked just now");
  });

  it("shows the backend's words otherwise", () => {
    const downloading = status({ state: "downloading", version: "0.2.0", percent: 40 }, "Downloading v0.2.0… 40%", SECONDS);
    expect(statusLine(downloading, NOW)).toBe("Downloading v0.2.0… 40%");
    const notSetUp = status({ state: "not_set_up" }, "Updates aren't set up for this build");
    expect(statusLine(notSetUp, NOW)).toBe("Updates aren't set up for this build");
  });
});

describe("isNewer", () => {
  it("keeps the latest revision", () => {
    const old = status({ state: "checking" }, "Checking…", null, 3);
    const later = status({ state: "up_to_date" }, "Up to date", null, 4);
    expect(isNewer(old, null)).toBe(true);
    expect(isNewer(later, old)).toBe(true);
    expect(isNewer(old, later)).toBe(false);
    expect(isNewer(later, later)).toBe(true);
  });
});
