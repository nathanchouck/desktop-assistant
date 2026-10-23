import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ResultItem } from "../bindings";

const commands = vi.hoisted(() => ({
  searchQuery: vi.fn(),
  itemActivate: vi.fn(),
}));
vi.mock("../bindings", () => ({ commands }));

const { SearchStore, SLOW_ACTION_MS } = await import("./search.svelte");

const row = (id: string, kind: ResultItem["kind"] = "file", extra: Partial<ResultItem> = {}): ResultItem => ({
  id,
  kind,
  title: id,
  subtitle: "",
  icon: null,
  actions: kind === "ask_ai" ? ["ask_ai"] : kind === "command" ? ["run"] : ["open"],
  accessory: null,
  confirm: null,
  ...extra,
});

/** A store that has answered one search with `items`. */
async function storeWith(items: ResultItem[], onAsk = vi.fn(), onSettings = vi.fn()) {
  commands.searchQuery.mockResolvedValueOnce({ status: "ok", data: { seq: 1, items } });
  const store = new SearchStore({ onAsk, onSettings });
  await store.setQuery("query");
  return store;
}

const reply = (data: unknown) => commands.itemActivate.mockResolvedValueOnce({ status: "ok", data });

/** The next search answers with `items` (echoing its sequence number). */
const answer = (items: ResultItem[]) =>
  commands.searchQuery.mockImplementationOnce(async (_query: string, seq: number) => ({
    status: "ok",
    data: { seq, items },
  }));

describe("SearchStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    commands.itemActivate.mockResolvedValue({ status: "ok", data: { type: "hide" } });
  });

  it("asks the default AI with the query", async () => {
    const onAsk = vi.fn();
    const store = await storeWith([row("a"), row("ask-ai", "ask_ai")], onAsk);
    store.ask();
    expect(onAsk).toHaveBeenCalledWith("query", null);
  });

  it("asks the AI a prefix names, without the prefix", async () => {
    const onAsk = vi.fn();
    const store = await storeWith([row("ask-ai", "ask_ai")], onAsk);
    answer([row("ask-ai", "ask_ai")]);
    await store.setQuery("g write a haiku");
    store.ask();
    expect(onAsk).toHaveBeenLastCalledWith("write a haiku", "chatgpt");
    await store.activate(0);
    expect(onAsk).toHaveBeenLastCalledWith("write a haiku", "chatgpt");
    answer([row("ask-ai", "ask_ai")]);
    await store.setQuery("c sort my files");
    store.ask();
    expect(onAsk).toHaveBeenLastCalledWith("sort my files", "claude");
  });

  it("asks the other AI with Shift+Tab", async () => {
    const onAsk = vi.fn();
    commands.searchQuery.mockResolvedValueOnce({ status: "ok", data: { seq: 1, items: [row("ask-ai", "ask_ai")] } });
    const store = new SearchStore({ onAsk, defaultProvider: () => "chatgpt" });
    await store.setQuery("what's up");
    expect(store.otherProvider()).toBe("claude");
    store.askOther();
    expect(onAsk).toHaveBeenCalledWith("what's up", "claude");
    // With a prefix, "other" is the AI the prefix didn't name.
    answer([row("ask-ai", "ask_ai")]);
    await store.setQuery("c what's up");
    store.askOther();
    expect(onAsk).toHaveBeenLastCalledWith("what's up", "chatgpt");
  });

  it("runs ChatGPT's fix when it can't answer, but not for the other AI", async () => {
    const onAsk = vi.fn();
    const store = await storeWith([row("cmd:codex-install", "command")], onAsk);
    store.ask(undefined, "chatgpt");
    expect(commands.itemActivate).toHaveBeenCalledWith("cmd:codex-install", "run");
    expect(onAsk).not.toHaveBeenCalled();
    // The fix row is ChatGPT's; asking Claude goes ahead.
    store.ask(undefined, "claude");
    expect(onAsk).toHaveBeenCalledWith("query", "claude");
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

  it("runs the first of a key's actions that the row offers", async () => {
    const store = await storeWith([row("clip:1", "clip", { actions: ["paste", "copy", "delete"] })]);
    reply({ type: "notice", message: "Copied" });
    await store.activate(0, ["copy_path", "copy"]);
    expect(commands.itemActivate).toHaveBeenCalledWith("clip:1", "copy");
    await store.activate(0, ["run_as_admin"]);
    expect(store.notice).toBe("That doesn't apply to clip:1.");
  });

  it("asks before a risky command and runs it on the second Enter", async () => {
    const store = await storeWith([row("sys:restart", "command", { confirm: "Restart your PC?" })]);
    await store.activate();
    expect(commands.itemActivate).not.toHaveBeenCalled();
    expect(store.notice).toBe("Restart your PC? Press Enter again to confirm.");
    await store.activate();
    expect(commands.itemActivate).toHaveBeenCalledWith("sys:restart", "run");
  });

  it("forgets a confirmation when the highlight moves", async () => {
    const store = await storeWith([row("sys:restart", "command", { confirm: "Restart?" }), row("b")]);
    await store.activate();
    store.move(1);
    store.move(-1);
    expect(store.notice).toBeNull();
    await store.activate();
    expect(commands.itemActivate).not.toHaveBeenCalled();
  });

  it("follows what Rust says to do next", async () => {
    const onAsk = vi.fn();
    const onSettings = vi.fn();
    const store = await storeWith([row("a")], onAsk, onSettings);

    reply({ type: "notice", message: "Pinned" });
    await store.activate();
    expect(store.notice).toBe("Pinned");

    reply({ type: "ask", prompt: "explain this", provider: null });
    await store.activate();
    expect(onAsk).toHaveBeenCalledWith("explain this", null);
    expect(store.query).toBe("");

    answer([row("a")]);
    await store.setQuery("query");
    answer([row("kill 1")]);
    reply({ type: "set_query", query: "kill " });
    await store.activate();
    expect(store.query).toBe("kill ");
    await vi.waitFor(() => expect(store.current?.id).toBe("kill 1"));

    reply({ type: "open_settings" });
    await store.activate();
    expect(onSettings).toHaveBeenCalled();
  });

  it("hands a reopened chat to the chat view and clears the search", async () => {
    const onOpenChat = vi.fn();
    commands.searchQuery.mockResolvedValueOnce({
      status: "ok",
      data: { seq: 1, items: [row("history:4", "conversation")] },
    });
    const store = new SearchStore({ onAsk: vi.fn(), onOpenChat });
    await store.setQuery("history");
    const chat = { conversation: 3, provider: "claude", model: "", entries: [{ kind: "user", text: "hi", context: null }] };
    reply({ type: "open_chat", chat });
    await store.activate();
    expect(commands.itemActivate).toHaveBeenCalledWith("history:4", "open");
    expect(onOpenChat).toHaveBeenCalledWith(chat);
    expect(store.query).toBe("");
    expect(store.items).toEqual([]);
  });

  it("starts a question about a file with Ctrl+Shift+A", async () => {
    const onCompose = vi.fn();
    commands.searchQuery.mockResolvedValueOnce({
      status: "ok",
      data: { seq: 1, items: [row("file:C:\\a.pdf", "file", { actions: ["open", "ask_about"] })] },
    });
    const store = new SearchStore({ onAsk: vi.fn(), onCompose });
    await store.setQuery("a.pdf");
    const attachments = [{ type: "path", path: "C:\\a.pdf", folder: false }];
    reply({ type: "compose", attachments });
    await store.activate(0, ["ask_about"]);
    expect(commands.itemActivate).toHaveBeenCalledWith("file:C:\\a.pdf", "ask_about");
    expect(onCompose).toHaveBeenCalledWith(attachments);
    expect(store.query).toBe("");
  });

  it("says a slow action is running, until it's done", async () => {
    vi.useFakeTimers();
    try {
      const store = await storeWith([row("script:backup.ps1", "script", { title: "Backup", actions: ["copy"] })]);
      let finish: (value: unknown) => void = () => {};
      commands.itemActivate.mockReturnValueOnce(new Promise((resolve) => (finish = resolve)));
      const running = store.activate();
      vi.advanceTimersByTime(SLOW_ACTION_MS - 1);
      expect(store.notice).toBeNull();
      vi.advanceTimersByTime(1);
      expect(store.notice).toBe("Running Backup…");
      finish({ status: "ok", data: { type: "notice", message: "Backup failed: disk full" } });
      await running;
      expect(store.notice).toBe("Backup failed: disk full");

      // A quick one never says so.
      reply({ type: "hide" });
      await store.activate();
      vi.advanceTimersByTime(SLOW_ACTION_MS * 2);
      expect(store.notice).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("refreshes in place, keeping the highlighted row", async () => {
    const store = await storeWith([row("a"), row("clip:2", "clip", { actions: ["paste", "pin"] })]);
    store.move(1);
    answer([row("clip:2", "clip", { accessory: "Pinned" }), row("a")]);
    reply({ type: "refresh" });
    await store.activate(1, ["pin"]);
    await vi.waitFor(() => expect(store.items[0]?.accessory).toBe("Pinned"));
    expect(store.current?.id).toBe("clip:2");
  });
});
