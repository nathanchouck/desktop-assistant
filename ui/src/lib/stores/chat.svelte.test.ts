import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AiStream } from "../bindings";

const commands = vi.hoisted(() => ({
  aiAsk: vi.fn(),
  aiFollowUp: vi.fn(),
  aiApprove: vi.fn(),
  aiStop: vi.fn(),
  aiClose: vi.fn(),
}));
vi.mock("../bindings", () => ({ commands }));

const { ChatStore } = await import("./chat.svelte");

const ok = <T>(data: T) => ({ status: "ok" as const, data });
const reply = (conversation: number, context: string | null = null) => ok({ conversation, context });
const text = (conversation: number, t: string): AiStream => ({
  conversation,
  event: { type: "text_delta", text: t },
});

/** A promise you resolve later, to hold a command "in flight". */
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

describe("ChatStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    commands.aiClose.mockResolvedValue(ok(null));
  });

  it("holds early events until the conversation id is known, dropping others", async () => {
    const answer = deferred<ReturnType<typeof reply>>();
    commands.aiAsk.mockReturnValue(answer.promise);
    const chat = new ChatStore();
    const asking = chat.ask("hi");
    chat.onEvent(text(1, "stale from the last chat"));
    chat.onEvent(text(2, "Hello"));
    answer.resolve(reply(2));
    await asking;
    chat.onEvent(text(2, " there"));
    expect(chat.transcript.entries).toEqual([
      { kind: "user", text: "hi" },
      { kind: "assistant", text: "Hello there" },
    ]);
  });

  it("ignores a reply that arrives after the chat was closed", async () => {
    const answer = deferred<ReturnType<typeof reply>>();
    commands.aiAsk.mockReturnValue(answer.promise);
    const chat = new ChatStore();
    const asking = chat.ask("hi");
    await chat.close();
    answer.resolve(reply(3));
    await asking;
    chat.onEvent(text(3, "late"));
    expect(chat.active).toBe(false);
    expect(chat.transcript.entries).toEqual([]);
  });

  it("starts the conversation on a follow-up if the first ask failed", async () => {
    commands.aiAsk.mockResolvedValueOnce({ status: "error", error: { message: "not installed" } });
    const chat = new ChatStore();
    await chat.ask("hi");
    expect(chat.transcript.running).toBe(false);
    expect(chat.transcript.entries[1]).toEqual({ kind: "error", text: "not installed" });

    commands.aiAsk.mockResolvedValueOnce(reply(5));
    chat.draft = "again";
    await chat.followUp();
    expect(commands.aiFollowUp).not.toHaveBeenCalled();
    expect(commands.aiAsk).toHaveBeenLastCalledWith("again");
    chat.onEvent(text(5, "ok"));
    expect(chat.transcript.entries.at(-1)).toEqual({ kind: "assistant", text: "ok" });
  });

  it("answers the pending approval", async () => {
    commands.aiAsk.mockResolvedValue(reply(1));
    commands.aiApprove.mockResolvedValue(ok(null));
    const chat = new ChatStore();
    await chat.ask("move it");
    chat.onEvent({
      conversation: 1,
      event: { type: "approval_requested", id: "r1", tool: "Bash", summary: "mv", detail: "mv a b" },
    });
    expect(chat.pending?.id).toBe("r1");
    await chat.answer(true);
    expect(commands.aiApprove).toHaveBeenCalledWith("r1", true);
  });

  it("shows what Claude was told about the screen under your message", async () => {
    commands.aiAsk.mockResolvedValue(reply(1, "Downloads · 2 selected"));
    commands.aiFollowUp.mockResolvedValue(reply(1, null));
    const chat = new ChatStore();
    await chat.ask("sort these");
    expect(chat.transcript.entries[0]).toEqual({
      kind: "user",
      text: "sort these",
      context: "Downloads · 2 selected",
    });
    chat.onEvent({ conversation: 1, event: { type: "done" } });
    chat.draft = "thanks";
    await chat.followUp();
    expect(chat.transcript.entries.at(-1)).toEqual({ kind: "user", text: "thanks" });
  });
});
