import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AiStream, OpenedChat } from "../bindings";

const commands = vi.hoisted(() => ({
  aiAsk: vi.fn(),
  aiFollowUp: vi.fn(),
  aiApprove: vi.fn(),
  aiStop: vi.fn(),
  aiClose: vi.fn(),
  aiCompareKeep: vi.fn(),
  chatCopyAnswer: vi.fn(),
  chatPasteAnswer: vi.fn(),
}));
vi.mock("../bindings", () => ({ commands }));

const { ChatStore } = await import("./chat.svelte");

const ok = <T>(data: T) => ({ status: "ok" as const, data });
const reply = (conversation: number, context: string | null = null, provider = "claude") =>
  ok({ conversation, context, provider });
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
    expect(commands.aiAsk).toHaveBeenLastCalledWith("again", null, []);
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

  it("carries on with one side of a comparison, mid-answer", async () => {
    commands.aiCompareKeep.mockResolvedValueOnce(ok(null));
    commands.aiFollowUp.mockResolvedValue(reply(8, null, "chatgpt"));
    const chat = new ChatStore();
    await chat.continueFrom(
      {
        provider: "chatgpt",
        conversation: 8,
        transcript: { entries: [{ kind: "user", text: "q" }], running: true, model: "gpt-6.1-sol" },
      },
      "typed",
    );
    expect(commands.aiCompareKeep).toHaveBeenCalledWith(8);
    expect(chat.active).toBe(true);
    expect(chat.provider).toBe("chatgpt");
    expect(chat.draft).toBe("typed");
    chat.onEvent(text(9, "the other AI's"));
    chat.onEvent(text(8, "rest of it"));
    chat.onEvent({ conversation: 8, event: { type: "done" } });
    expect(chat.transcript.entries.at(-1)).toEqual({ kind: "assistant", text: "rest of it" });
    // Follow-ups go on in that conversation.
    await chat.followUp();
    expect(commands.aiFollowUp).toHaveBeenCalledWith("typed", []);

    commands.aiCompareKeep.mockResolvedValueOnce({ status: "error", error: { message: "gone" } });
    await chat.continueFrom({ provider: "claude", conversation: 3, transcript: { entries: [], running: true, model: "" } });
    expect(chat.transcript.entries.at(-1)).toEqual({ kind: "error", text: "gone" });
    expect(chat.transcript.running).toBe(false);
  });

  it("asks the AI it was told to, and learns who answers", async () => {
    commands.aiAsk.mockResolvedValueOnce(reply(1, null, "chatgpt"));
    const chat = new ChatStore();
    const asking = chat.ask("hi", "chatgpt");
    expect(chat.provider).toBe("chatgpt");
    await asking;
    expect(commands.aiAsk).toHaveBeenCalledWith("hi", "chatgpt", []);

    // The default AI: unknown until the backend says.
    commands.aiAsk.mockResolvedValueOnce(reply(2, null, "claude"));
    const second = chat.ask("hello");
    expect(chat.provider).toBeNull();
    await second;
    expect(commands.aiAsk).toHaveBeenLastCalledWith("hello", null, []);
    expect(chat.provider).toBe("claude");

    await chat.close();
    expect(chat.provider).toBeNull();
  });

  it("retries a failed first question with the same AI", async () => {
    commands.aiAsk.mockResolvedValueOnce({ status: "error", error: { message: "Codex isn't installed" } });
    const chat = new ChatStore();
    await chat.ask("hi", "chatgpt");
    commands.aiAsk.mockResolvedValueOnce(reply(3, null, "chatgpt"));
    chat.draft = "again";
    await chat.followUp();
    expect(commands.aiAsk).toHaveBeenLastCalledWith("again", "chatgpt", []);
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

  it("sends attachments with a question and with a follow-up", async () => {
    const picture = { type: "image" as const, id: "1a", width: 4, height: 3 };
    const selected = { type: "text" as const, text: "teh cat" };
    commands.aiAsk.mockResolvedValue(reply(1, "Picture · Notepad"));
    commands.aiFollowUp.mockResolvedValue(reply(1, "Selected text"));
    const chat = new ChatStore();
    await chat.ask("what's this?", null, [picture]);
    expect(commands.aiAsk).toHaveBeenCalledWith("what's this?", null, [picture]);
    expect(chat.transcript.entries[0]).toEqual({
      kind: "user",
      text: "what's this?",
      context: "Picture · Notepad",
    });
    chat.onEvent({ conversation: 1, event: { type: "done" } });
    chat.draft = "and this?";
    await chat.followUp([selected]);
    expect(commands.aiFollowUp).toHaveBeenCalledWith("and this?", [selected]);
    expect(chat.transcript.entries.at(-1)).toEqual({ kind: "user", text: "and this?", context: "Selected text" });
  });
  describe("a reopened chat (from history)", () => {
    const saved = (conversation = 7): OpenedChat => ({
      conversation,
      provider: "chatgpt",
      model: "gpt-6.1-sol",
      entries: [
        { kind: "user", text: "plan a trip", context: "Desktop" },
        { kind: "assistant", text: "Kyoto in April." },
      ],
    });

    it("shows the saved transcript, idle, with who answered", () => {
      const chat = new ChatStore();
      chat.draft = "leftover";
      chat.reopen(saved());
      expect(chat.active).toBe(true);
      expect(chat.provider).toBe("chatgpt");
      expect(chat.draft).toBe("");
      expect(chat.transcript).toEqual({
        entries: [
          { kind: "user", text: "plan a trip", context: "Desktop" },
          { kind: "assistant", text: "Kyoto in April." },
        ],
        running: false,
        model: "gpt-6.1-sol",
      });
      expect(chat.finishedAnswer).toBe("Kyoto in April.");
    });

    it("carries the same conversation on with a follow-up", async () => {
      commands.aiFollowUp.mockResolvedValue(reply(7, null, "chatgpt"));
      const chat = new ChatStore();
      chat.reopen(saved());
      chat.draft = "and in May?";
      await chat.followUp();
      expect(commands.aiAsk).not.toHaveBeenCalled();
      expect(commands.aiFollowUp).toHaveBeenCalledWith("and in May?", []);
      expect(chat.transcript.running).toBe(true);
      expect(chat.finishedAnswer).toBeNull();
      // Its events are this conversation's; another's are ignored.
      chat.onEvent(text(6, "stale"));
      chat.onEvent(text(7, "Also nice."));
      chat.onEvent({ conversation: 7, event: { type: "done" } });
      expect(chat.transcript.entries.at(-1)).toEqual({ kind: "assistant", text: "Also nice." });
      expect(chat.finishedAnswer).toBe("Also nice.");
    });

    it("drops what an earlier ask was still waiting for", async () => {
      const answer = deferred<ReturnType<typeof reply>>();
      commands.aiAsk.mockReturnValue(answer.promise);
      const chat = new ChatStore();
      const asking = chat.ask("hi");
      chat.reopen(saved(9));
      answer.resolve(reply(8));
      await asking;
      chat.onEvent(text(8, "late"));
      expect(chat.transcript.entries).toHaveLength(2);
      chat.onEvent({ conversation: 9, event: { type: "tool_started", id: "t1", name: "Read", summary: "" } });
      expect(chat.transcript.entries).toHaveLength(3);
    });
  });

  describe("the last answer", () => {
    async function answered(): Promise<InstanceType<typeof ChatStore>> {
      commands.aiAsk.mockResolvedValue(reply(1));
      const chat = new ChatStore();
      await chat.ask("what's new?");
      chat.onEvent(text(1, "**Three** things."));
      return chat;
    }

    it("is there only once the turn is over", async () => {
      const chat = await answered();
      expect(chat.finishedAnswer).toBeNull();
      expect(await chat.copyAnswer()).toBeNull();
      expect(await chat.pasteAnswer()).toBeNull();
      expect(commands.chatCopyAnswer).not.toHaveBeenCalled();
      expect(commands.chatPasteAnswer).not.toHaveBeenCalled();
      chat.onEvent({ conversation: 1, event: { type: "done" } });
      expect(chat.finishedAnswer).toBe("**Three** things.");
    });

    it("is copied or pasted as Markdown", async () => {
      commands.chatCopyAnswer.mockResolvedValue(ok(null));
      commands.chatPasteAnswer.mockResolvedValue(ok(null));
      const chat = await answered();
      chat.onEvent({ conversation: 1, event: { type: "done" } });
      expect(await chat.copyAnswer()).toBeNull();
      expect(commands.chatCopyAnswer).toHaveBeenCalledWith("**Three** things.");
      expect(await chat.pasteAnswer()).toBeNull();
      expect(commands.chatPasteAnswer).toHaveBeenCalledWith("**Three** things.");
    });

    it("reports a failure", async () => {
      commands.chatPasteAnswer.mockResolvedValue({ status: "error", error: { message: "couldn't paste" } });
      const chat = await answered();
      chat.onEvent({ conversation: 1, event: { type: "done" } });
      expect(await chat.pasteAnswer()).toBe("couldn't paste");
    });
  });
});
