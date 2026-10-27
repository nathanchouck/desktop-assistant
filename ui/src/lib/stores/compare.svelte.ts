import { commands, type AiStream, type Attachment, type CompareReply, type ProviderId } from "../bindings";
import { applyEvent, noteContext, pendingApproval, type Entry, type Transcript } from "../chat";
import { COMPARED } from "../compare";

/** One AI's column in a comparison. */
export interface Side {
  provider: ProviderId;
  /** Its conversation in the backend; null until it starts, or if it couldn't. */
  conversation: number | null;
  transcript: Transcript;
}

/** An approval card waiting for Approve / Deny, and the column it's in. */
export interface Waiting {
  side: number;
  approval: Extract<Entry, { kind: "approval" }>;
}

/** A column handed over to carry on as a normal chat (Ctrl+1 / Ctrl+2). */
export interface Released {
  side: { provider: ProviderId; conversation: number; transcript: Transcript };
  /** The follow-up being typed, kept for the chat. */
  draft: string;
}

const asking = (text: string): Transcript => ({ entries: [{ kind: "user", text }], running: true, model: "" });

/** The same question to Claude and ChatGPT, answered side by side. */
export class CompareStore {
  /** The comparison is showing (instead of search results). */
  active = $state(false);
  /** Claude's column, then ChatGPT's. */
  sides = $state<Side[]>([]);
  /** The follow-up being typed (Enter sends it to both). */
  draft = $state("");
  /** Approval requests in the order they came: [column, request id]. */
  #asked = $state<[number, string][]>([]);

  /** A turn is running on either side. */
  running = $derived(this.sides.some((s) => s.transcript.running));
  /** The approval Ctrl+Y / Ctrl+N answer: the oldest one still waiting. */
  pending = $derived(this.#oldestWaiting());

  /** Events that arrived while `aiCompare` was in flight (ids still unknown). */
  #buffer: AiStream[] | null = null;
  /** Bumped by `start`, `close` and `release`, so a late reply can tell it's stale. */
  #generation = 0;

  /** Asks both AIs `text` (with `attachments`) and shows their columns. */
  async start(text: string, attachments: Attachment[] = []): Promise<void> {
    const generation = ++this.#generation;
    this.active = true;
    this.draft = "";
    this.#asked = [];
    this.sides = COMPARED.map((provider) => ({ provider, conversation: null, transcript: asking(text) }));
    // Events can beat `aiCompare`'s reply; hold them until we know the ids.
    this.#buffer = [];
    const res = await commands.aiCompare(text, attachments);
    if (generation !== this.#generation) return;
    const held = this.#buffer ?? [];
    this.#buffer = null;
    if (res.status === "error") {
      this.sides.forEach((_, i) => this.#fail(i, res.error.message));
      return;
    }
    this.#apply(res.data);
    for (const message of held) this.onEvent(message);
  }

  /**
   * Sends the draft to both AIs (each that's still in the comparison),
   * with `attachments` if any. If neither got the first question, this one
   * starts the comparison again.
   */
  async followUp(attachments: Attachment[] = []): Promise<void> {
    const text = this.draft.trim();
    if (!text || this.running) return;
    const asked = this.sides.filter((s) => s.conversation !== null);
    if (asked.length === 0) return this.start(text, attachments);
    const generation = this.#generation;
    this.draft = "";
    for (const side of asked) {
      side.transcript.entries.push({ kind: "user", text });
      side.transcript.running = true;
    }
    const res = await commands.aiCompareFollowUp(text, attachments);
    if (generation !== this.#generation) return;
    if (res.status === "error") {
      this.sides.forEach((side, i) => {
        if (side.conversation !== null) this.#fail(i, res.error.message);
      });
      return;
    }
    this.#apply(res.data);
  }

  /** Handles an event from the backend for either column. */
  onEvent(message: AiStream): void {
    if (!this.active) return;
    if (this.#buffer) {
      this.#buffer.push(message);
      return;
    }
    const at = this.sides.findIndex((s) => s.conversation === message.conversation);
    const side = this.sides[at];
    if (!side) return;
    if (message.event.type === "approval_requested") this.#asked.push([at, message.event.id]);
    applyEvent(side.transcript, message.event);
  }

  /**
   * Approves or denies the oldest waiting approval, or the one in column
   * `side` (its own buttons).
   */
  async answer(allow: boolean, side?: number): Promise<void> {
    const waiting = side === undefined ? this.pending : this.#waitingIn(side);
    if (!waiting) return;
    const res = await commands.aiApprove(waiting.approval.id, allow);
    if (res.status === "error") this.#fail(waiting.side, res.error.message);
  }

  /** Stops both answers; the comparison stays open. */
  async stop(): Promise<void> {
    await commands.aiStop();
  }

  /** Ends both conversations and returns to search. */
  async close(): Promise<void> {
    this.#reset();
    await commands.aiClose();
  }

  /**
   * Leaves the comparison with column `index`, to carry on with that AI
   * as a normal chat (the caller hands it to the chat). Null when that AI
   * isn't in the comparison (it couldn't start).
   */
  release(index: number): Released | null {
    const side = this.sides[index];
    if (side === undefined || side.conversation === null) return null;
    const released: Released = {
      side: {
        provider: side.provider,
        conversation: side.conversation,
        transcript: $state.snapshot(side.transcript),
      },
      draft: this.draft,
    };
    this.#reset();
    return released;
  }

  #reset(): void {
    this.#generation++;
    this.#buffer = null;
    this.#asked = [];
    this.active = false;
    this.sides = [];
    this.draft = "";
  }

  /** Takes in what the backend said about each side. */
  #apply(reply: CompareReply): void {
    for (const answer of reply.sides) {
      const at = this.sides.findIndex((s) => s.provider === answer.provider);
      const side = this.sides[at];
      if (!side) continue;
      if (answer.conversation !== null) side.conversation = answer.conversation;
      if (answer.error) this.#fail(at, answer.error);
      else noteContext(side.transcript, answer.context);
    }
  }

  #oldestWaiting(): Waiting | null {
    for (const [side, id] of this.#asked) {
      const waiting = this.#waitingIn(side);
      if (waiting?.approval.id === id) return waiting;
    }
    // Not seen arriving (it shouldn't happen): left to right.
    for (const side of this.sides.keys()) {
      const waiting = this.#waitingIn(side);
      if (waiting) return waiting;
    }
    return null;
  }

  /** Column `side`'s approval waiting for an answer, while its turn runs. */
  #waitingIn(side: number): Waiting | null {
    const transcript = this.sides[side]?.transcript;
    if (!transcript?.running) return null;
    const approval = pendingApproval(transcript);
    return approval ? { side, approval } : null;
  }

  #fail(side: number, message: string): void {
    const transcript = this.sides[side]?.transcript;
    if (!transcript) return;
    transcript.entries.push({ kind: "error", text: message });
    transcript.running = false;
  }
}
