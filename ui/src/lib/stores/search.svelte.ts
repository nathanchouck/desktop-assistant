import { commands, type ResultItem } from "../bindings";
import { LatestOnly } from "../latest";

/** State for the search box and results list. */
export class SearchStore {
  query = $state("");
  items = $state<ResultItem[]>([]);
  selected = $state(0);
  notice = $state<string | null>(null);

  /** Bar + results when there is text; just the bar otherwise. */
  expanded = $derived(this.query.trim().length > 0);

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

  /** Enter: run the selected item's default action. */
  async activate(index = this.selected): Promise<void> {
    const item = this.items[index];
    if (!item) return;
    switch (item.kind) {
      case "command": {
        const res = await commands.commandRun(item.id);
        if (res.status === "error") this.notice = res.error.message;
        else this.clear();
        break;
      }
      case "ask_ai":
        this.notice = "Asking AI arrives in milestone 4.";
        break;
      default:
        this.notice = `Opening ${item.kind} items arrives in a later milestone.`;
    }
  }
}
