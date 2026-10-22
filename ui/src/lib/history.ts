import type { HistoryOverview } from "./bindings";

/** The "Chats kept" stepper's range. */
export const KEEP_MIN = 50;
export const KEEP_MAX = 2000;
export const KEEP_STEP = 50;

const FORGET_CHOICES: [number, string][] = [
  [0, "Never"],
  [30, "30 days"],
  [90, "90 days"],
  [365, "1 year"],
];

/**
 * The "Forget chats after" choices as [value, text]; a number of days set
 * by hand in the settings file is listed too, so it shows as chosen.
 */
export function forgetOptions(current: number): [string, string][] {
  const options = FORGET_CHOICES.map(([days, text]): [string, string] => [String(days), text]);
  if (!FORGET_CHOICES.some(([days]) => days === current)) {
    options.push([String(current), current === 1 ? "1 day" : `${current} days`]);
    options.sort((a, b) => (a[0] === "0" ? -1 : b[0] === "0" ? 1 : Number(a[0]) - Number(b[0])));
  }
  return options;
}

/** "12 chats saved, 2 pinned." */
export function historySummary(overview: HistoryOverview): string {
  const { conversations: n, pinned } = overview;
  if (n === 0) return "No saved chats.";
  const saved = n === 1 ? "1 chat saved" : `${n} chats saved`;
  return pinned > 0 ? `${saved}, ${pinned} pinned.` : `${saved}.`;
}

/** What Clear removes: everything but pinned chats. 0 means nothing to clear. */
export function clearable(overview: HistoryOverview): number {
  return Math.max(0, overview.conversations - overview.pinned);
}
