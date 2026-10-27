import { commands, type AiStream, type Attachment, type OpenedChat, type ProviderId } from "../bindings";
import { applyEvent, fromSaved, lastAnswer, noteContext, pendingApproval, type Transcript } from "../chat";

const empty = (): Transcript => ({ entries: [], running: false, model: "" });

/** The conversation with Claude or ChatGPT shown in the launcher. */
export class ChatStore {
  /** The chat view is showing (instead of search results). */
  active = $state(false);
  transcript = $state<Transcript>(empty());
  /** The follow-up being typed. */
  draft = $state("");
  /** Who answers; null for the default AI until the backend says which. */
  provider = $state<ProviderId | null>(null);

  pending = $derived(pendingApproval(this.transcript));
  /**
   * The finished answer that Ctrl+Shift+C copies and Ctrl+Enter pastes;
   * null while a turn runs or before any answer.
   */
  finishedAnswer = $derived(this.transcript.running ? null : lastAnswer(this.transcript));

  /** The backend's id for the conversation on screen; null before it starts. */
  #conversation: number | null = null;
  /** Events that arrived while `aiAsk` was in flight (its id still unknown). */
  #buffer: AiStream[] | null = null;
  /** Bumped by `ask` and `close`, so a late reply can tell it's stale. */
  #generation = 0;
  /** The AI asked for (null: the default), kept for a retry. */
  #requested: ProviderId | null = null;

  /**
   * Starts a new conversation with `provider` (or the default AI) and shows
   * it. `attachments` (selected text, a file, a picture) go with the question.
   */
  async ask(text: string, provider: ProviderId | null = null, attachments: Attachment[] = []): Promise<void> {
    this.#generation++;
    this.#conversation = null;
    this.#requested = provider;
    this.provider = provider;
    this.active = true;
    this.draft = "";
    this.transcript = { entries: [{ kind: "user", text }], running: true, model: "" };
    await this.#start(text, attachments);
  }

  /**
   * Shows a saved conversation (from chat history). The backend has made it
   * the current one, so a follow-up carries it on.
   */
  reopen(saved: OpenedChat): void {
    this.#generation++;
    this.#conversation = saved.conversation;
    this.#buffer = null;
    this.#requested = saved.provider;
    this.provider = saved.provider;
    this.active = true;
    this.draft = "";
    this.transcript = { entries: fromSaved(saved.entries), running: false, model: saved.model };
  }

  /**
   * Carries on with one side of a comparison as this chat (Ctrl+1 /
   * Ctrl+2): its conversation and what it said so far, even mid-answer.
   * The backend ends the other AI's conversation.
   */
  async continueFrom(
    side: { provider: ProviderId; conversation: number; transcript: Transcript },
    draft = "",
  ): Promise<void> {
    this.#generation++;
    this.#conversation = side.conversation;
    this.#buffer = null;
    this.#requested = side.provider;
    this.provider = side.provider;
    this.active = true;
    this.draft = draft;
    this.transcript = side.transcript;
    const res = await commands.aiCompareKeep(side.conversation);
    if (res.status === "error") this.#fail(res.error.message);
  }

  /** Copies the last answer; returns an error message if that failed. */
  async copyAnswer(): Promise<string | null> {
    const text = this.finishedAnswer;
    if (text === null) return null;
    const res = await commands.chatCopyAnswer(text);
    return res.status === "error" ? res.error.message : null;
  }

  /**
   * Pastes the last answer into the window the launcher was opened from
   * (the launcher hides); returns an error message if that failed.
   */
  async pasteAnswer(): Promise<string | null> {
    const text = this.finishedAnswer;
    if (text === null) return null;
    const res = await commands.chatPasteAnswer(text);
    return res.status === "error" ? res.error.message : null;
  }

  /** Sends the draft as a follow-up, with `attachments` if any. */
  async followUp(attachments: Attachment[] = []): Promise<void> {
    const text = this.draft.trim();
    if (!text || this.transcript.running) return;
    this.draft = "";
    this.transcript.entries.push({ kind: "user", text });
    this.transcript.running = true;
    // If the first question never reached the AI (say it wasn't installed
    // yet), this one starts the conversation instead.
    if (this.#conversation === null) return this.#start(text, attachments);
    const res = await commands.aiFollowUp(text, attachments);
    if (res.status === "error") return this.#fail(res.error.message);
    this.provider = res.data.provider;
    noteContext(this.transcript, res.data.context);
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
    this.#requested = null;
    this.provider = null;
    this.active = false;
    this.transcript = empty();
    this.draft = "";
    await commands.aiClose();
  }

  async #start(text: string, attachments: Attachment[]): Promise<void> {
    const generation = this.#generation;
    // Events can beat `aiAsk`'s reply; hold them until we know the id.
    this.#buffer = [];
    const res = await commands.aiAsk(text, this.#requested, attachments);
    if (generation !== this.#generation) return;
    const held = this.#buffer ?? [];
    this.#buffer = null;
    if (res.status === "error") return this.#fail(res.error.message);
    this.#conversation = res.data.conversation;
    this.provider = res.data.provider;
    noteContext(this.transcript, res.data.context);
    for (const message of held) this.onEvent(message);
  }

  #fail(message: string): void {
    this.transcript.entries.push({ kind: "error", text: message });
    this.transcript.running = false;
  }
}
