import { commands, type Action, type ResultItem, type SearchUpdated } from "../bindings";
import { LatestOnly } from "../latest";

/** State for the search box and results list. */
export class SearchStore {
  query = $state("");
  items = $state<ResultItem[]>([]);
  selected = $state(0);
  notice = $state<string | null>(null);

  /** Bar + results when there is text; just the bar otherwise. */
  expanded = $derived(this.query.trim().length > 0);
  current = $derived<ResultItem | undefined>(this.items[this.selected]);

  #latest = new LatestOnly();
  #onAsk: (text: string) => void;

  /** `onAsk` gets the query when it should go to the AI. */
  constructor(onAsk: (text: string) => void) {
    this.#onAsk = onAsk;
  }

  async setQuery(query: string): Promise<void> {
    this.query = query;
    this.notice = null;
    if (query.trim() === "") {
      this.clear();
      return;
    }
    const seq = this.#latest.next();
    const res = await commands.searchQuery(query, seq);
    if (res.status === "error") {
      if (this.#latest.accept(seq)) this.notice = res.error.message;
      return;
    }
    if (!this.#latest.accept(res.data.seq)) return;
    this.items = res.data.items;
    this.selected = 0;
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
  }

  move(delta: number): void {
    const n = this.items.length;
    if (n === 0) return;
    this.selected = (this.selected + delta + n) % n;
  }

  /** Sends the query to the AI, whatever the results. */
  ask(text = this.query): void {
    const question = text.trim();
    if (question) this.#onAsk(question);
  }

  /** Runs `action` (default: the item's first action) on row `index`. */
  async activate(index = this.selected, action?: Action): Promise<void> {
    const item = this.items[index];
    if (!item) return;
    if (item.kind === "ask_ai") return this.ask();
    const chosen = action ?? item.actions[0];
    if (!chosen || !item.actions.includes(chosen)) {
      this.notice = `That doesn't apply to ${item.title}.`;
      return;
    }
    const res = await commands.itemActivate(item.id, chosen);
    if (res.status === "error") this.notice = res.error.message;
    else this.clear();
  }
}
