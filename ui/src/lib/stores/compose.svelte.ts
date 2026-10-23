import type { Attachment } from "../bindings";
import { mergeAttachments } from "../compose";

/**
 * A question being put together with attachments (the selected text, a
 * file, a snip), shown as chips above the input. While it has any and no
 * chat is open, the input takes the question instead of a search.
 */
export class ComposeStore {
  attachments = $state<Attachment[]>([]);
  /** The question being typed. */
  draft = $state("");

  active = $derived(this.attachments.length > 0);

  /** Adds attachments, skipping ones already there. */
  add(attachments: Attachment[]): void {
    this.attachments = mergeAttachments(this.attachments, attachments);
  }

  /** Backspace in an empty box: drops the last chip. */
  removeLast(): void {
    this.attachments = this.attachments.slice(0, -1);
    if (this.attachments.length === 0) this.draft = "";
  }

  remove(index: number): void {
    this.attachments = this.attachments.filter((_, i) => i !== index);
    if (this.attachments.length === 0) this.draft = "";
  }

  clear(): void {
    this.attachments = [];
    this.draft = "";
  }

  /** The attachments, handed over to be sent (the chips go away). */
  take(): Attachment[] {
    const taken = $state.snapshot(this.attachments) as Attachment[];
    this.clear();
    return taken;
  }
}
