import type { Action } from "./bindings";

/** What a key press in the search box means. Pure, so it's easy to test. */
export type Intent =
  | { type: "move"; delta: number }
  /**
   * Run a result. `actions` lists what the key can mean, best first; the
   * first one the result offers runs. Unset means its default action.
   */
  | { type: "activate"; index?: number; actions?: Action[] }
  /**
   * Send the query to the AI, whatever the results: the default one (Tab),
   * or the other one (`other`, Shift+Tab).
   */
  | { type: "ask"; other?: true }
  /** Ask Claude and ChatGPT side by side (Ctrl+Tab). */
  | { type: "compare" }
  /** Open the settings menu (Ctrl+,). */
  | { type: "settings" }
  /** Open the action menu for the selected result (Ctrl+K). */
  | { type: "menu" }
  | { type: "escape" };

interface KeyLike {
  key: string;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}

/**
 * The fixed keys for a result's other actions: [key, what it can mean].
 * The first of those a result offers is what the key does for it.
 */
export const ACTION_KEYS: [string, Action[]][] = [
  ["Ctrl ↵", ["reveal_in_explorer", "paste"]],
  // Ctrl+Shift+Enter is the Windows convention for "run as administrator".
  ["Ctrl ⇧ ↵", ["run_as_admin"]],
  ["Ctrl ⇧ C", ["copy_path", "copy"]],
  ["Ctrl ⇧ P", ["pin"]],
  ["Ctrl ⇧ A", ["ask_about"]],
  ["Ctrl D", ["delete", "trash"]],
];

const actionsFor = (key: string): Action[] => ACTION_KEYS.find(([k]) => k === key)?.[1] ?? [];

export function intentFor(e: KeyLike): Intent | null {
  const ctrlOnly = e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey;
  const ctrlShift = e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey;
  switch (e.key) {
    case "ArrowDown":
      return { type: "move", delta: 1 };
    case "ArrowUp":
      return { type: "move", delta: -1 };
    case "Enter":
      if (ctrlShift) return { type: "activate", actions: actionsFor("Ctrl ⇧ ↵") };
      if (ctrlOnly) return { type: "activate", actions: actionsFor("Ctrl ↵") };
      return { type: "activate" };
    case "Tab":
      if (ctrlOnly) return { type: "compare" };
      if (e.ctrlKey || e.altKey || e.metaKey) return null;
      return e.shiftKey ? { type: "ask", other: true } : { type: "ask" };
    case "Escape":
      return { type: "escape" };
  }
  if (ctrlOnly && e.key === ",") return { type: "settings" };
  if (ctrlShift) {
    switch (e.key.toLowerCase()) {
      case "c":
        return { type: "activate", actions: actionsFor("Ctrl ⇧ C") };
      case "p":
        return { type: "activate", actions: actionsFor("Ctrl ⇧ P") };
      case "a":
        return { type: "activate", actions: actionsFor("Ctrl ⇧ A") };
    }
  }
  if (ctrlOnly) {
    switch (e.key.toLowerCase()) {
      case "d":
        return { type: "activate", actions: actionsFor("Ctrl D") };
      case "k":
        return { type: "menu" };
    }
  }
  // Ctrl+1..9 opens that row directly.
  if (ctrlOnly && /^[1-9]$/.test(e.key)) {
    return { type: "activate", index: Number(e.key) - 1 };
  }
  return null;
}

/**
 * The key that runs `action` on a result offering `actions`: Enter for its
 * default, else the fixed key that picks it. `null` when only the action
 * menu has it.
 */
export function keyFor(actions: Action[], action: Action): string | null {
  if (actions[0] === action) return "↵";
  for (const [key, meanings] of ACTION_KEYS) {
    if (meanings.find((a) => actions.includes(a)) === action) return key;
  }
  return null;
}

/** A footer key hint: [key, label]. */
export type Hint = [string, string];

/** Short footer labels for a result's extra actions, in display order. */
const hintLabels: [Action, string][] = [
  ["reveal_in_explorer", "Show in folder"],
  ["paste", "Paste"],
  ["copy_path", "Copy path"],
  ["copy", "Copy"],
  ["run_as_admin", "Admin"],
  ["ask_about", "Ask"],
  ["pin", "Pin"],
  ["delete", "Delete"],
  ["trash", "Delete"],
];

/** What Enter does, by a result's default action. */
const enterLabels: Record<Action, string> = {
  open: "Open",
  run: "Run",
  run_as_admin: "Admin",
  reveal_in_explorer: "Show",
  copy_path: "Copy path",
  copy: "Copy",
  paste: "Paste",
  delete: "Delete",
  end: "End",
  pin: "Pin",
  ask_ai: "Ask",
  ask_about: "Ask",
  open_with: "Open with",
  trash: "Delete",
};

/** At most this many extra-action hints, so the footer fits. */
const MAX_ACTION_HINTS = 2;

/** The action menu's hint, shown when a result has more than one action. */
export const MENU_HINT: Hint = ["Ctrl K", "Actions"];

/** Ctrl+Tab's hint: ask Claude and ChatGPT side by side. */
const COMPARE_HINT: Hint = ["Ctrl Tab", "Ask both"];

/**
 * Footer hints while searching, for the selected result's actions. On the
 * "Ask" row, `otherAi` names the AI that Shift+Tab asks instead.
 * `compare`: "offered" when the list has the row that asks both AIs (the
 * Ask row then mentions Ctrl+Tab), "selected" when that row is selected.
 */
export function searchHints(
  actions: Action[],
  otherAi?: string,
  compare: "no" | "offered" | "selected" = "no",
): Hint[] {
  const [first] = actions;
  if (compare === "selected") return [["↵", "Ask both"], ["Esc", "Clear"]];
  if (first === "ask_ai") {
    const other: Hint[] = otherAi ? [["⇧ Tab", `Ask ${otherAi}`]] : [];
    const both: Hint[] = compare === "offered" ? [COMPARE_HINT] : [];
    return [["↵", "Ask"], ...other, ...both, ["Esc", "Clear"]];
  }
  const shownKeys = new Set<string>(["↵"]);
  const extra = hintLabels
    .filter(([action]) => actions.includes(action))
    .map(([action, label]): [string | null, string] => [keyFor(actions, action), label])
    // Only actions with their own key; two sharing a key show once.
    .filter((hint): hint is Hint => hint[0] !== null && !shownKeys.has(hint[0]) && Boolean(shownKeys.add(hint[0])))
    .slice(0, MAX_ACTION_HINTS);
  const menu: Hint[] = actions.length > 1 ? [MENU_HINT] : [];
  return [["↵", first ? enterLabels[first] : "Open"], ...extra, ...menu, ["Esc", "Clear"]];
}

/**
 * Footer hints while chatting. `answer`: there's a finished answer to
 * paste (Ctrl+Enter) or copy (Ctrl+Shift+C).
 */
export function chatHints(state: { running: boolean; pending: boolean; answer?: boolean }): Hint[] {
  if (state.pending) return [["Ctrl Y", "Approve"], ["Ctrl N", "Deny"]];
  if (state.running) return [["Esc", "Stop"]];
  const answer: Hint[] = state.answer
    ? [
        ["Ctrl ↵", "Paste"],
        ["Ctrl ⇧ C", "Copy"],
      ]
    : [];
  return [["↵", "Send"], ...answer, ["Esc", "Close"]];
}

/** What a key press means while the chat is showing. */
export type ChatIntent =
  | { type: "send" }
  /** Answer the pending approval. */
  | { type: "approve"; allow: boolean }
  /** Stop the answer in progress (Ctrl+.). */
  | { type: "stop" }
  /** Paste the last answer into the window you came from (Ctrl+Enter). */
  | { type: "paste_answer" }
  /** Copy the last answer (Ctrl+Shift+C). */
  | { type: "copy_answer" }
  /** Scroll the conversation; `pages` false means one line. */
  | { type: "scroll"; delta: number; pages: boolean }
  | { type: "escape" };

export function chatIntentFor(e: KeyLike): ChatIntent | null {
  const plain = !e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey;
  const ctrlOnly = e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey;
  const ctrlShift = e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey;
  switch (e.key) {
    case "Enter":
      if (plain) return { type: "send" };
      return ctrlOnly ? { type: "paste_answer" } : null;
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
  if (ctrlShift && e.key.toLowerCase() === "c") return { type: "copy_answer" };
  return null;
}

/** What a key press means while Claude and ChatGPT answer side by side. */
export type CompareIntent =
  /** Send the follow-up to both. */
  | { type: "send" }
  /** Answer the oldest waiting approval (Ctrl+Y / Ctrl+N). */
  | { type: "approve"; allow: boolean }
  /** Stop both answers (Ctrl+.). */
  | { type: "stop" }
  /** Carry on with one AI as a normal chat: 0 Claude (Ctrl+1), 1 ChatGPT (Ctrl+2). */
  | { type: "keep"; side: number }
  /** Scroll both columns; `pages` false means one line. */
  | { type: "scroll"; delta: number; pages: boolean }
  | { type: "escape" };

export function compareIntentFor(e: KeyLike): CompareIntent | null {
  const ctrlOnly = e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey;
  if (ctrlOnly && (e.key === "1" || e.key === "2")) return { type: "keep", side: Number(e.key) - 1 };
  const intent = chatIntentFor(e);
  // Which answer to paste or copy is ambiguous here; Ctrl+1 / Ctrl+2 first.
  if (intent?.type === "paste_answer" || intent?.type === "copy_answer") return null;
  return intent;
}

/**
 * Footer hints while comparing. `pending` names the AI whose approval
 * Ctrl+Y answers; `names` are the two AIs, left to right.
 */
export function compareHints(state: { running: boolean; pending: string | null; names: string[] }): Hint[] {
  if (state.pending) return [["Ctrl Y", `Approve ${state.pending}`], ["Ctrl N", "Deny"]];
  const keep = state.names.slice(0, 2).map((name, i): Hint => [`Ctrl ${i + 1}`, `${name} only`]);
  if (state.running) return [...keep, ["Esc", "Stop"]];
  return [["↵", "Ask both"], ...keep, ["Esc", "Close"]];
}

/** What a key press means while a question with attachments is being typed. */
export type ComposeIntent =
  /** Ask the default AI (Enter, Tab), or the other one (Shift+Tab). */
  | { type: "ask"; other?: true }
  /** Ask Claude and ChatGPT side by side (Ctrl+Tab). */
  | { type: "compare" }
  /** Backspace in an empty box: drop the last attachment. */
  | { type: "remove_last" }
  /** Drop the attachments and the question. */
  | { type: "escape" };

export function composeIntentFor(e: KeyLike, draftEmpty: boolean): ComposeIntent | null {
  const plain = !e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey;
  const ctrlOnly = e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey;
  switch (e.key) {
    case "Enter":
      return plain ? { type: "ask" } : null;
    case "Tab":
      if (ctrlOnly) return { type: "compare" };
      if (e.ctrlKey || e.altKey || e.metaKey) return null;
      return e.shiftKey ? { type: "ask", other: true } : { type: "ask" };
    case "Backspace":
      return plain && draftEmpty ? { type: "remove_last" } : null;
    case "Escape":
      return { type: "escape" };
  }
  return null;
}

/**
 * Footer hints while a question with attachments is being typed.
 * `compare`: mention that Ctrl+Tab asks both AIs.
 */
export function composeHints(ai: string, otherAi: string, draftEmpty: boolean, compare = false): Hint[] {
  const remove: Hint[] = draftEmpty ? [["⌫", "Remove"]] : [];
  const both: Hint[] = compare ? [COMPARE_HINT] : [];
  return [["↵", `Ask ${ai}`], ["⇧ Tab", `Ask ${otherAi}`], ...both, ...remove, ["Esc", "Clear"]];
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
