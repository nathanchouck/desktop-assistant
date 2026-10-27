import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AiEvent, AiStream, Attachment, CompareSide } from "../bindings";

const commands = vi.hoisted(() => ({
  aiCompare: vi.fn(),
  aiCompareFollowUp: vi.fn(),
  aiApprove: vi.fn(),
  aiStop: vi.fn(),
  aiClose: vi.fn(),
}));
vi.mock("../bindings", () => ({ commands }));

const { CompareStore } = await import("./compare.svelte");

const ok = <T>(data: T) => ({ status: "ok" as const, data });
const side = (provider: "claude" | "chatgpt", conversation: number | null, extra: Partial<CompareSide> = {}) => ({
  provider,
  conversation,
  context: null,
  error: null,
  ...extra,
});
/** Claude answers as conversation 1, ChatGPT as 2. */
const bothAsked = (extra: { claude?: Partial<CompareSide>; chatgpt?: Partial<CompareSide> } = {}) =>
  ok({ sides: [side("claude", 1, extra.claude), side("chatgpt", 2, extra.chatgpt)] });
const event = (conversation: number, e: AiEvent): AiStream => ({ conversation, event: e });
const text = (conversation: number, t: string) => event(conversation, { type: "text_delta", text: t });
const done = (conversation: number) => event(conversation, { type: "done" });
const approval = (conversation: number, id: string) =>
  event(conversation, { type: "approval_requested", id, tool: "Bash", summary: "", detail: "rm x" });

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

/** A comparison of "q" where both AIs started (1: Claude, 2: ChatGPT). */
async function started() {
  commands.aiCompare.mockResolvedValueOnce(bothAsked());
  const store = new CompareStore();
  await store.start("q");
  return store;
}

describe("CompareStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    commands.aiClose.mockResolvedValue(ok(null));
    commands.aiApprove.mockResolvedValue(ok(null));
  });

  it("asks both, holding early events until the ids are known", async () => {
    const reply = deferred<ReturnType<typeof bothAsked>>();
    commands.aiCompare.mockReturnValue(reply.promise);
    const store = new CompareStore();
    const attachments: Attachment[] = [{ type: "text", text: "teh" }];
    const asking = store.start("which is faster?", attachments);
    expect(store.active).toBe(true);
    expect(store.sides.map((s) => s.provider)).toEqual(["claude", "chatgpt"]);
    expect(store.running).toBe(true);
    store.onEvent(text(2, "Go"));
    store.onEvent(text(7, "a stale chat"));
    reply.resolve(bothAsked({ claude: { context: "Downloads" } }));
    await asking;
    expect(commands.aiCompare).toHaveBeenCalledWith("which is faster?", attachments);
    store.onEvent(text(1, "Rust"));
    expect(store.sides[0]?.transcript.entries).toEqual([
      { kind: "user", text: "which is faster?", context: "Downloads" },
      { kind: "assistant", text: "Rust" },
    ]);
    expect(store.sides[1]?.transcript.entries).toEqual([
      { kind: "user", text: "which is faster?" },
      { kind: "assistant", text: "Go" },
    ]);

    store.onEvent(done(1));
    expect(store.running).toBe(true);
    store.onEvent(done(2));
    expect(store.running).toBe(false);
  });

  it("sends follow-ups to both once both are done", async () => {
    const store = await started();
    store.draft = "and memory?";
    await store.followUp();
    expect(commands.aiCompareFollowUp).not.toHaveBeenCalled();
    expect(store.draft).toBe("and memory?");

    store.onEvent(done(1));
    store.onEvent(done(2));
    commands.aiCompareFollowUp.mockResolvedValueOnce(bothAsked());
    await store.followUp();
    expect(commands.aiCompareFollowUp).toHaveBeenCalledWith("and memory?", []);
    expect(store.draft).toBe("");
    for (const s of store.sides) {
      expect(s.transcript.entries.at(-1)).toEqual({ kind: "user", text: "and memory?" });
      expect(s.transcript.running).toBe(true);
    }
  });

  it("shows why one AI couldn't answer and carries on with the other", async () => {
    commands.aiCompare.mockResolvedValueOnce(
      ok({ sides: [side("claude", 1), side("chatgpt", null, { error: "Codex isn't installed" })] }),
    );
    const store = new CompareStore();
    await store.start("q");
    expect(store.sides[1]?.transcript.entries.at(-1)).toEqual({ kind: "error", text: "Codex isn't installed" });
    expect(store.sides[1]?.transcript.running).toBe(false);
    store.onEvent(done(1));

    commands.aiCompareFollowUp.mockResolvedValueOnce(ok({ sides: [side("claude", 1)] }));
    store.draft = "more";
    await store.followUp();
    expect(store.sides[0]?.transcript.running).toBe(true);
    expect(store.sides[1]?.transcript.entries.at(-1)?.kind).toBe("error");
    expect(store.release(1)).toBeNull();
  });

  it("starts over when neither AI got the question", async () => {
    commands.aiCompare.mockResolvedValueOnce({ status: "error", error: { message: "picture gone" } });
    const store = new CompareStore();
    await store.start("q");
    expect(store.sides.every((s) => s.transcript.entries.at(-1)?.kind === "error")).toBe(true);
    commands.aiCompare.mockResolvedValueOnce(bothAsked());
    store.draft = "again";
    await store.followUp();
    expect(commands.aiCompare).toHaveBeenLastCalledWith("again", []);
    expect(commands.aiCompareFollowUp).not.toHaveBeenCalled();
  });

  it("answers the oldest approval first, labeled by its column", async () => {
    const store = await started();
    store.onEvent(approval(2, "g1"));
    store.onEvent(approval(1, "c1"));
    expect(store.pending?.side).toBe(1);
    await store.answer(true);
    expect(commands.aiApprove).toHaveBeenLastCalledWith("g1", true);
    store.onEvent(event(2, { type: "approval_resolved", id: "g1", allowed: true }));
    expect(store.pending?.side).toBe(0);
    expect(store.pending?.approval.id).toBe("c1");

    // A column's own buttons answer its own card.
    store.onEvent(approval(2, "g2"));
    await store.answer(false, 1);
    expect(commands.aiApprove).toHaveBeenLastCalledWith("g2", false);

    // A turn that ended leaves nothing to answer.
    store.onEvent(done(1));
    store.onEvent(event(2, { type: "approval_resolved", id: "g2", allowed: false }));
    expect(store.pending).toBeNull();
  });

  it("shows a failed approval in its column", async () => {
    const store = await started();
    store.onEvent(approval(1, "c1"));
    commands.aiApprove.mockResolvedValueOnce({ status: "error", error: { message: "no longer pending" } });
    await store.answer(true);
    expect(store.sides[0]?.transcript.entries.at(-1)).toEqual({ kind: "error", text: "no longer pending" });
  });

  it("hands one column over to carry on as a chat", async () => {
    const store = await started();
    store.onEvent(text(2, "half"));
    store.draft = "typed";
    const kept = store.release(1);
    expect(kept).toEqual({
      side: {
        provider: "chatgpt",
        conversation: 2,
        transcript: {
          entries: [
            { kind: "user", text: "q" },
            { kind: "assistant", text: "half" },
          ],
          running: true,
          model: "",
        },
      },
      draft: "typed",
    });
    expect(store.active).toBe(false);
    expect(store.sides).toEqual([]);
    // The backend ends the other side; the comparison isn't closed here.
    expect(commands.aiClose).not.toHaveBeenCalled();
    store.onEvent(text(2, " more"));
    expect(kept?.side.transcript.entries.at(-1)).toEqual({ kind: "assistant", text: "half" });
  });

  it("stops both, and closes, ignoring a late reply", async () => {
    const store = await started();
    await store.stop();
    expect(commands.aiStop).toHaveBeenCalled();

    const reply = deferred<ReturnType<typeof bothAsked>>();
    commands.aiCompare.mockReturnValue(reply.promise);
    const asking = store.start("again");
    await store.close();
    reply.resolve(bothAsked());
    await asking;
    store.onEvent(text(1, "late"));
    expect(commands.aiClose).toHaveBeenCalled();
    expect(store.active).toBe(false);
    expect(store.sides).toEqual([]);
  });
});
