import { commands, type Action, type ResultItem } from "../bindings";
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

  /** Runs `action` (default: the item's first action) on row `index`. */
  async activate(index = this.selected, action?: Action): Promise<void> {
    const item = this.items[index];
    if (!item) return;
    if (item.kind === "ask_ai") {
      this.notice = "Asking AI arrives in milestone 4.";
      return;
    }
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
