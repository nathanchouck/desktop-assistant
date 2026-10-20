import { commands, type ResultItem } from "../bindings";
import { LatestOnly } from "../latest";

/** State for the search box and results list. */
export class SearchStore {
  query = $state("");
  items = $state<ResultItem[]>([]);
  selected = $state(0);
  notice = $state<string | null>(null);

  #latest = new LatestOnly();

  async setQuery(query: string): Promise<void> {
    this.query = query;
    this.notice = null;
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
