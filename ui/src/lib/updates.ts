import type { UpdatePhase, UpdateStatus } from "./bindings";

/** Whether "Check now" can start a check: set up, and nothing under way. */
export function canCheck(phase: UpdatePhase): boolean {
  switch (phase.state) {
    case "idle":
    case "up_to_date":
    case "ready":
    case "failed":
      return true;
    case "not_set_up":
    case "checking":
    case "downloading":
    case "installing":
      return false;
  }
}

/** The downloaded version "Install & restart" would install, if any. */
export function readyVersion(phase: UpdatePhase): string | null {
  return phase.state === "ready" ? phase.version : null;
}

const plural = (n: number, unit: string) => `${n} ${unit}${n === 1 ? "" : "s"}`;

/** "checked 5 minutes ago": `lastChecked` is seconds since 1970. */
export function checkedAgo(lastChecked: number | null, nowMs: number): string {
  if (lastChecked === null) return "not checked on this PC yet";
  // A clock set back would give a time in the future: call it "just now".
  const seconds = Math.max(0, Math.floor(nowMs / 1000) - lastChecked);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);
  if (minutes < 1) return "checked just now";
  if (hours < 1) return `checked ${plural(minutes, "minute")} ago`;
  if (days < 1) return `checked ${plural(hours, "hour")} ago`;
  if (days === 1) return "checked yesterday";
  return `checked ${days} days ago`;
}

/** The line under the version: what the updater is doing, and when it last heard from GitHub. */
export function statusLine(status: UpdateStatus, nowMs: number): string {
  const { phase, summary, last_checked } = status;
  switch (phase.state) {
    case "idle": {
      // Not checked since the app started, but maybe in an earlier run.
      if (last_checked === null) return summary;
      const ago = checkedAgo(last_checked, nowMs);
      return ago.charAt(0).toUpperCase() + ago.slice(1);
    }
    case "up_to_date":
      return `${summary} · ${checkedAgo(last_checked, nowMs)}`;
    default:
      return summary;
  }
}

/**
 * Whether `next` should replace `current`. A command's reply can arrive
 * after newer events, so the higher revision wins.
 */
export function isNewer(next: UpdateStatus, current: UpdateStatus | null): boolean {
  return current === null || next.revision >= current.revision;
}
