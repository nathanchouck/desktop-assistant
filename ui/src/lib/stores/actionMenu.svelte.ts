import type { Action, ResultItem } from "../bindings";
import { filterEntries, menuEntries, type MenuEntry } from "../actions";

/** The action menu (Ctrl+K): every action the selected result offers. */
export class ActionMenuStore {
  /** The result whose actions are listed; `null` while closed. */
  item = $state<ResultItem | null>(null);
  filter = $state("");
  selected = $state(0);

  open = $derived(this.item !== null);
  entries = $derived<MenuEntry[]>(this.item ? filterEntries(menuEntries(this.item), this.filter) : []);

  show(item: ResultItem): void {
    this.item = item;
    this.filter = "";
    this.selected = 0;
  }

  close(): void {
    this.item = null;
    this.filter = "";
    this.selected = 0;
  }

  setFilter(text: string): void {
    this.filter = text;
    this.selected = 0;
  }

  move(delta: number): void {
    const n = this.entries.length;
    if (n === 0) return;
    this.selected = (this.selected + delta + n) % n;
  }

  /** The highlighted action (or row `index`'s); closes the menu. */
  choose(index = this.selected): Action | null {
    const action = this.entries[index]?.action ?? null;
    if (action) this.close();
    return action;
  }
}
