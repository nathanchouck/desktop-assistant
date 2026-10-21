import { commands, type AiStream } from "../bindings";
import { applyEvent, pendingApproval, type Transcript } from "../chat";

const empty = (): Transcript => ({ entries: [], running: false, model: "" });

/** The conversation with Claude shown in the launcher. */
export class ChatStore {
  /** The chat view is showing (instead of search results). */
  active = $state(false);
  transcript = $state<Transcript>(empty());
  /** The follow-up being typed. */
  draft = $state("");

  pending = $derived(pendingApproval(this.transcript));

  /** The backend's id for the conversation on screen; null before it starts. */
  #conversation: number | null = null;
  /** Events that arrived while `aiAsk` was in flight (its id still unknown). */
  #buffer: AiStream[] | null = null;
  /** Bumped by `ask` and `close`, so a late reply can tell it's stale. */
  #generation = 0;

  /** Starts a new conversation and shows it. */
  async ask(text: string): Promise<void> {
    this.#generation++;
    this.#conversation = null;
    this.active = true;
    this.draft = "";
    this.transcript = { entries: [{ kind: "user", text }], running: true, model: "" };
    await this.#start(text);
  }

  async followUp(): Promise<void> {
    const text = this.draft.trim();
    if (!text || this.transcript.running) return;
    this.draft = "";
    this.transcript.entries.push({ kind: "user", text });
    this.transcript.running = true;
    // If the first question never reached Claude (say it wasn't installed
    // yet), this one starts the conversation instead.
    if (this.#conversation === null) return this.#start(text);
    const res = await commands.aiFollowUp(text);
    if (res.status === "error") this.#fail(res.error.message);
  }

  /** Handles an event from the backend for whichever conversation is shown. */
  onEvent(message: AiStream): void {
    if (this.#buffer) {
      this.#buffer.push(message);
      return;
    }
    if (message.conversation !== this.#conversation) return;
    applyEvent(this.transcript, message.event);
  }

  async answer(allow: boolean): Promise<void> {
    const approval = this.pending;
    if (!approval) return;
    const res = await commands.aiApprove(approval.id, allow);
    if (res.status === "error") this.#fail(res.error.message);
  }

  async stop(): Promise<void> {
    await commands.aiStop();
  }

  /** Ends the conversation and returns to search. */
  async close(): Promise<void> {
    this.#generation++;
    this.#conversation = null;
    this.#buffer = null;
    this.active = false;
    this.transcript = empty();
    this.draft = "";
    await commands.aiClose();
  }

  async #start(text: string): Promise<void> {
    const generation = this.#generation;
    // Events can beat `aiAsk`'s reply; hold them until we know the id.
    this.#buffer = [];
    const res = await commands.aiAsk(text);
    if (generation !== this.#generation) return;
    const held = this.#buffer ?? [];
    this.#buffer = null;
    if (res.status === "error") return this.#fail(res.error.message);
    this.#conversation = res.data;
    for (const message of held) this.onEvent(message);
  }

  #fail(message: string): void {
    this.transcript.entries.push({ kind: "error", text: message });
    this.transcript.running = false;
  }
}
