import {
  commands,
  type Action,
  type Activation,
  type Attachment,
  type OpenedChat,
  type ProviderId,
  type ResultItem,
  type SearchUpdated,
} from "../bindings";
import { confirmQuestion } from "../actions";
import { forcedProvider, otherProvider } from "../chat";
import { COMPARE_ROW_ID, comparePrefix } from "../compare";
import { LatestOnly } from "../latest";

/** Rows that stand in for "Ask …" while that AI can't answer yet. */
const FIX_ROWS: Record<ProviderId, string[]> = {
  claude: ["cmd:claude-install", "cmd:claude-sign-in"],
  chatgpt: ["cmd:codex-install", "cmd:chatgpt-sign-in"],
};

/** A risky command waits this long for Enter again. */
export const CONFIRM_MS = 4000;
/** An action still running after this long says so in the footer. */
export const SLOW_ACTION_MS = 400;

/** What the store needs from the rest of the app. */
export interface SearchHooks {
  /** The query (or a result's prompt) should go to the AI: `provider`, or the default one. */
  onAsk: (text: string, provider: ProviderId | null) => void;
  /** Show the settings menu. */
  onSettings?: () => void;
  /** Show a saved conversation (chat history) in the chat view. */
  onOpenChat?: (chat: OpenedChat) => void;
  /** Start a question with these attached (Ctrl+Shift+A on a file). */
  onCompose?: (attachments: Attachment[]) => void;
  /** The query should go to Claude and ChatGPT side by side. */
  onCompare?: (text: string) => void;
  /** The AI that Tab and the "Ask" row use. */
  defaultProvider?: () => ProviderId;
}

/** State for the search box and results list. */
export class SearchStore {
  query = $state("");
  items = $state<ResultItem[]>([]);
  selected = $state(0);
  notice = $state<string | null>(null);

  /** Bar + results when there is text; just the bar otherwise. */
  expanded = $derived(this.query.trim().length > 0);
  current = $derived<ResultItem | undefined>(this.items[this.selected]);
  /** The list has the row that asks both AIs (both are signed in). */
  compareOffered = $derived(this.items.some((item) => item.id === COMPARE_ROW_ID));

  #latest = new LatestOnly();
  #hooks: Required<SearchHooks>;
  /** An action waiting for Enter again (see `confirmQuestion`). */
  #armed: { id: string; action: Action; until: number } | null = null;

  constructor(hooks: SearchHooks) {
    this.#hooks = {
      onSettings: () => {},
      onOpenChat: () => {},
      onCompose: () => {},
      onCompare: () => {},
      defaultProvider: () => "claude",
      ...hooks,
    };
  }

  async setQuery(query: string, keepSelection = false): Promise<void> {
    this.query = query;
    this.notice = null;
    this.#armed = null;
    if (query.trim() === "") {
      this.clear();
      return;
    }
    const selectedId = keepSelection ? this.current?.id : undefined;
    const seq = this.#latest.next();
    const res = await commands.searchQuery(query, seq);
    if (res.status === "error") {
      if (this.#latest.accept(seq)) this.notice = res.error.message;
      return;
    }
    if (!this.#latest.accept(res.data.seq)) return;
    this.items = res.data.items;
    const kept = res.data.items.findIndex((item) => item.id === selectedId);
    this.selected = kept >= 0 ? kept : 0;
  }

  /** Fuller results for a search (files that took longer than the rest). */
  onUpdate({ seq, items }: SearchUpdated): void {
    if (!this.#latest.acceptUpdate(seq)) return;
    // If you moved the highlight, keep it on the same row; otherwise the
    // (possibly new) top row is highlighted.
    const selectedId = this.selected > 0 ? this.items[this.selected]?.id : undefined;
    this.items = items;
    const kept = items.findIndex((item) => item.id === selectedId);
    this.selected = kept >= 0 ? kept : 0;
  }

  clear(): void {
    this.#latest.invalidate();
    this.query = "";
    this.items = [];
    this.selected = 0;
    this.notice = null;
    this.#armed = null;
  }

  move(delta: number): void {
    const n = this.items.length;
    if (n === 0) return;
    this.selected = (this.selected + delta + n) % n;
    this.#disarm();
  }

  /**
   * Sends the query to the AI, whatever the results: `provider`, else the
   * one a `c `/`g ` prefix names, else the default one.
   */
  ask(text = this.query, provider: ProviderId | null = null): void {
    const forced = forcedProvider(text);
    const question = (forced?.text ?? text).trim();
    if (!question) return;
    const target = provider ?? forced?.provider ?? null;
    // When that AI can't answer yet, the last row offers the fix instead
    // (install or sign in): run that. Without a target, the row is the
    // default AI's.
    const fixes = target ? FIX_ROWS[target] : [...FIX_ROWS.claude, ...FIX_ROWS.chatgpt];
    const last = this.items.length - 1;
    if (fixes.includes(this.items[last]?.id ?? "")) {
      void this.activate(last);
      return;
    }
    this.#hooks.onAsk(question, target);
  }

  /** The AI that Shift+Tab asks: the one Tab wouldn't. */
  otherProvider(): ProviderId {
    return otherProvider(forcedProvider(this.query)?.provider ?? this.#hooks.defaultProvider());
  }

  /** Shift+Tab. */
  askOther(): void {
    this.ask(this.query, this.otherProvider());
  }

  /**
   * Ctrl+Tab, the comparison row, or "both <question>": asks Claude and
   * ChatGPT side by side, whatever the results (without any AI prefix).
   */
  compare(text = this.query): void {
    const question = (comparePrefix(text) ?? forcedProvider(text)?.text ?? text).trim();
    if (!question) return;
    // When an AI can't answer yet, the last row offers the fix instead
    // (install or sign in): run that.
    const fixes = [...FIX_ROWS.claude, ...FIX_ROWS.chatgpt];
    const last = this.items.length - 1;
    if (fixes.includes(this.items[last]?.id ?? "")) {
      void this.activate(last);
      return;
    }
    this.#hooks.onCompare(question);
  }

  /**
   * Runs an action on row `index`: the first of `actions` the row offers,
   * or its default action. A risky action asks first; Enter (or its own
   * key) again within `CONFIRM_MS` runs it.
   */
  async activate(index = this.selected, actions?: Action[]): Promise<void> {
    const item = this.items[index];
    if (!item) return;
    if (item.kind === "ask_ai") return item.id === COMPARE_ROW_ID ? this.compare() : this.ask();
    const armed = this.#takeArmed(item.id);
    // Enter confirms whatever is waiting, even if it isn't the default.
    const chosen = actions
      ? actions.find((a) => item.actions.includes(a))
      : (armed ?? item.actions[0]);
    if (!chosen) {
      this.notice = `That doesn't apply to ${item.title}.`;
      return;
    }
    const question = confirmQuestion(item, chosen);
    if (question && armed !== chosen) {
      this.selected = index;
      this.#armed = { id: item.id, action: chosen, until: Date.now() + CONFIRM_MS };
      const again = chosen === item.actions[0] ? "Press Enter again" : "Press Enter";
      this.notice = `${question} ${again} to confirm.`;
      return;
    }
    // Most actions are instant; a script that copies its output can take
    // seconds, so say it's running rather than seem stuck.
    const query = this.query;
    const slow = setTimeout(() => {
      if (this.query === query) this.notice = `Running ${item.title}…`;
    }, SLOW_ACTION_MS);
    const res = await commands.itemActivate(item.id, chosen);
    clearTimeout(slow);
    if (res.status === "error") this.notice = res.error.message;
    else this.#follow(res.data, item.id);
  }

  /** Does what's left after Rust ran the action on row `id`. */
  #follow(activation: Activation, id: string): void {
    switch (activation.type) {
      case "hide":
      case "copy":
      case "paste":
      case "open_url":
        this.clear();
        break;
      case "set_query":
        void this.setQuery(activation.query);
        break;
      case "ask":
        this.clear();
        this.#hooks.onAsk(activation.prompt, activation.provider);
        break;
      case "notice":
        this.notice = activation.message;
        break;
      case "refresh":
        void this.setQuery(this.query, true);
        break;
      case "removed":
        this.#remove(id);
        this.notice = activation.message;
        break;
      case "open_settings":
        this.clear();
        this.#hooks.onSettings();
        break;
      case "open_chat":
        this.clear();
        this.#hooks.onOpenChat(activation.chat);
        break;
      case "compose":
        this.clear();
        this.#hooks.onCompose(activation.attachments);
        break;
    }
  }

  /** The action armed on row `id`, if it's still waiting; disarms. */
  #takeArmed(id: string): Action | null {
    const armed = this.#armed;
    this.#armed = null;
    return armed !== null && armed.id === id && Date.now() <= armed.until ? armed.action : null;
  }

  /** Takes row `id` out of the list, keeping the highlight in place. */
  #remove(id: string): void {
    const at = this.items.findIndex((item) => item.id === id);
    if (at < 0) return;
    this.items = this.items.filter((_, i) => i !== at);
    if (this.selected > at || this.selected >= this.items.length) {
      this.selected = Math.max(0, this.selected - 1);
    }
  }

  #disarm(): void {
    if (this.#armed) {
      this.#armed = null;
      this.notice = null;
    }
  }
}
