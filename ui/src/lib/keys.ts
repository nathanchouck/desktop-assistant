import type { Action } from "./bindings";

/** What a key press in the search box means. Pure, so it's easy to test. */
export type Intent =
  | { type: "move"; delta: number }
  /** Run a result. `action` unset means its default (first) action. */
  | { type: "activate"; index?: number; action?: Action }
  | { type: "escape" };

interface KeyLike {
  key: string;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}

export function intentFor(e: KeyLike): Intent | null {
  const ctrlOnly = e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey;
  const ctrlShift = e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey;
  switch (e.key) {
    case "ArrowDown":
      return { type: "move", delta: 1 };
    case "ArrowUp":
      return { type: "move", delta: -1 };
    case "Enter":
      // Ctrl+Shift+Enter is the Windows convention for "run as administrator".
      if (ctrlShift) return { type: "activate", action: "run_as_admin" };
      if (ctrlOnly) return { type: "activate", action: "reveal_in_explorer" };
      return { type: "activate" };
    case "Escape":
      return { type: "escape" };
  }
  if (ctrlShift && e.key.toLowerCase() === "c") {
    return { type: "activate", action: "copy_path" };
  }
  // Ctrl+1..9 opens that row directly.
  if (ctrlOnly && /^[1-9]$/.test(e.key)) {
    return { type: "activate", index: Number(e.key) - 1 };
  }
  return null;
}

/** Footer hints for a result's extra actions, in display order. */
export const actionHints: [Action, string, string][] = [
  ["reveal_in_explorer", "Ctrl ↵", "Show in folder"],
  ["copy_path", "Ctrl ⇧ C", "Copy path"],
  ["run_as_admin", "Ctrl ⇧ ↵", "Admin"],
];
