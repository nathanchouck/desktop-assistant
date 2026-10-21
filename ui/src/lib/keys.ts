import type { Action } from "./bindings";

/** What a key press in the search box means. Pure, so it's easy to test. */
export type Intent =
  | { type: "move"; delta: number }
  /** Run a result. `action` unset means its default (first) action. */
  | { type: "activate"; index?: number; action?: Action }
  /** Send the query to the AI, whatever the results. */
  | { type: "ask" }
  /** Open the settings menu (Ctrl+,). */
  | { type: "settings" }
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
    case "Tab":
      return e.shiftKey || e.ctrlKey || e.altKey || e.metaKey ? null : { type: "ask" };
    case "Escape":
      return { type: "escape" };
  }
  if (ctrlOnly && e.key === ",") return { type: "settings" };
  if (ctrlShift && e.key.toLowerCase() === "c") {
    return { type: "activate", action: "copy_path" };
  }
  // Ctrl+1..9 opens that row directly.
  if (ctrlOnly && /^[1-9]$/.test(e.key)) {
    return { type: "activate", index: Number(e.key) - 1 };
  }
  return null;
}

/** A footer key hint: [key, label]. */
export type Hint = [string, string];

/** Hints for a result's extra actions, in display order. */
const actionHints: [Action, Hint][] = [
  ["reveal_in_explorer", ["Ctrl ↵", "Show in folder"]],
  ["copy_path", ["Ctrl ⇧ C", "Copy path"]],
  ["run_as_admin", ["Ctrl ⇧ ↵", "Admin"]],
];

/** Footer hints while searching, for the selected result's actions. */
export function searchHints(actions: Action[]): Hint[] {
  if (actions.includes("ask_ai")) return [["↵", "Ask"], ["Esc", "Clear"]];
  return [
    ["↵", "Open"],
    ...actionHints.filter(([action]) => actions.includes(action)).map(([, hint]) => hint),
    ["Esc", "Clear"],
  ];
}

/** Footer hints while chatting. */
export function chatHints(state: { running: boolean; pending: boolean }): Hint[] {
  if (state.pending) return [["Ctrl Y", "Approve"], ["Ctrl N", "Deny"]];
  if (state.running) return [["Esc", "Stop"]];
  return [["↵", "Send"], ["Esc", "Close"]];
}

/** What a key press means while the chat is showing. */
export type ChatIntent =
  | { type: "send" }
  /** Answer the pending approval. */
  | { type: "approve"; allow: boolean }
  /** Stop the answer in progress (Ctrl+.). */
  | { type: "stop" }
  /** Scroll the conversation; `pages` false means one line. */
  | { type: "scroll"; delta: number; pages: boolean }
  | { type: "escape" };

export function chatIntentFor(e: KeyLike): ChatIntent | null {
  const plain = !e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey;
  const ctrlOnly = e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey;
  switch (e.key) {
    case "Enter":
      return plain ? { type: "send" } : null;
    case "Escape":
      return { type: "escape" };
    case "ArrowUp":
      return { type: "scroll", delta: -1, pages: false };
    case "ArrowDown":
      return { type: "scroll", delta: 1, pages: false };
    case "PageUp":
      return { type: "scroll", delta: -1, pages: true };
    case "PageDown":
      return { type: "scroll", delta: 1, pages: true };
  }
  if (ctrlOnly) {
    switch (e.key.toLowerCase()) {
      case "y":
        return { type: "approve", allow: true };
      case "n":
        return { type: "approve", allow: false };
      case ".":
        return { type: "stop" };
    }
  }
  return null;
}

interface ComboLike {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

const MODIFIER_CODES = /^(Control|Alt|Shift|Meta|OS)(Left|Right)?$/;

/**
 * The shortcut a key press makes, in the form the settings file uses
 * ("Ctrl+Space", "Alt+Shift+K"). `null` while only modifiers are down, or
 * for a plain key without modifiers (except F1 to F24), which would be
 * typed everywhere else.
 */
export function shortcutFromEvent(e: ComboLike): string | null {
  if (MODIFIER_CODES.test(e.code)) return null;
  const key = e.code.replace(/^Key(?=[A-Z]$)/, "").replace(/^Digit(?=\d$)/, "");
  const mods = [
    e.ctrlKey && "Ctrl",
    e.altKey && "Alt",
    e.shiftKey && "Shift",
    e.metaKey && "Super",
  ].filter((m): m is string => Boolean(m));
  if (mods.length === 0 && !/^F([1-9]|1\d|2[0-4])$/.test(key)) return null;
  return [...mods, key].join("+");
}
