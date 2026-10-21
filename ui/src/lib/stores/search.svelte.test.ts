import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ResultItem } from "../bindings";

const commands = vi.hoisted(() => ({
  searchQuery: vi.fn(),
  itemActivate: vi.fn(),
}));
vi.mock("../bindings", () => ({ commands }));

const { SearchStore } = await import("./search.svelte");

const row = (id: string, kind: ResultItem["kind"] = "file"): ResultItem => ({
  id,
  kind,
  title: id,
  subtitle: "",
  icon: null,
  actions: kind === "ask_ai" ? ["ask_ai"] : kind === "command" ? ["run"] : ["open"],
});

/** A store that has answered one search with `items`. */
async function storeWith(items: ResultItem[], onAsk = vi.fn()) {
  commands.searchQuery.mockResolvedValueOnce({ status: "ok", data: { seq: 1, items } });
  const store = new SearchStore(onAsk);
  await store.setQuery("query");
  return store;
}

describe("SearchStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    commands.itemActivate.mockResolvedValue({ status: "ok", data: null });
  });

  it("asks Claude with the query", async () => {
    const onAsk = vi.fn();
    const store = await storeWith([row("a"), row("ask-ai", "ask_ai")], onAsk);
    store.ask();
    expect(onAsk).toHaveBeenCalledWith("query");
  });

  it("runs the sign-in fix instead when Claude can't answer", async () => {
    const onAsk = vi.fn();
    const store = await storeWith([row("a"), row("cmd:claude-sign-in", "command")], onAsk);
    store.ask();
    expect(onAsk).not.toHaveBeenCalled();
    expect(commands.itemActivate).toHaveBeenCalledWith("cmd:claude-sign-in", "run");
  });

  it("takes late results for the current search, keeping a moved highlight", async () => {
    const store = await storeWith([row("a"), row("b"), row("ask-ai", "ask_ai")]);
    store.move(1);
    store.onUpdate({ seq: 1, items: [row("file"), row("a"), row("b"), row("ask-ai", "ask_ai")] });
    expect(store.items.map((i) => i.id)).toEqual(["file", "a", "b", "ask-ai"]);
    expect(store.current?.id).toBe("b");
  });

  it("highlights the new top row when you hadn't moved", async () => {
    const store = await storeWith([row("ask-ai", "ask_ai")]);
    store.onUpdate({ seq: 1, items: [row("file"), row("ask-ai", "ask_ai")] });
    expect(store.current?.id).toBe("file");
  });

  it("ignores late results after the query is cleared", async () => {
    const store = await storeWith([row("ask-ai", "ask_ai")]);
    store.clear();
    store.onUpdate({ seq: 1, items: [row("file")] });
    expect(store.items).toEqual([]);
  });
});
