import type { Action, ResultItem } from "./bindings";
import { keyFor } from "./keys";

/** What each action is called in the action menu (Ctrl+K). */
export const ACTION_LABELS: Record<Action, string> = {
  open: "Open",
  run: "Run",
  run_as_admin: "Run as Administrator",
  reveal_in_explorer: "Show in Folder",
  copy_path: "Copy Path",
  copy: "Copy",
  paste: "Paste",
  delete: "Delete",
  end: "End Process",
  pin: "Pin / Unpin",
  ask_ai: "Ask AI",
  ask_about: "Ask AI About This",
  open_with: "Open With…",
  trash: "Delete to Recycle Bin",
};

/** One line of the action menu. */
export interface MenuEntry {
  action: Action;
  label: string;
  /** The key that runs it without the menu, if there is one. */
  key: string | null;
}

/** Everything `item` offers, in its own order (the default first). */
export function menuEntries(item: ResultItem): MenuEntry[] {
  return item.actions.map((action) => ({
    action,
    label: ACTION_LABELS[action],
    key: keyFor(item.actions, action),
  }));
}

/**
 * The entries whose label has every typed word, each at the start of a
 * word ("rec" finds "Delete to Recycle Bin", "op w" finds "Open With…").
 */
export function filterEntries(entries: MenuEntry[], text: string): MenuEntry[] {
  const words = text.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return entries;
  return entries.filter((entry) => {
    const labelWords = entry.label.toLowerCase().split(/[\s/]+/);
    return words.every((w) => labelWords.some((l) => l.startsWith(w)));
  });
}

/**
 * The question to ask before `action` runs on `item`, if any. A row's own
 * `confirm` covers its default action only (restart, end a process); some
 * actions always ask, whichever row they're on.
 */
export function confirmQuestion(item: ResultItem, action: Action): string | null {
  if (action === "trash") return `Move ${item.title} to the Recycle Bin?`;
  if (item.confirm && action === item.actions[0]) return item.confirm;
  return null;
}
