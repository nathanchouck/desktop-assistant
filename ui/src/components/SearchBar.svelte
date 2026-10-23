<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import type { Attachment } from "../lib/bindings";
  import { chipLabel, chipName, pictureKey } from "../lib/compose";
  import { GLYPHS } from "../lib/glyphs";

  interface Props {
    value: string;
    /** Chatting: the box takes a follow-up instead of a search. */
    chat?: boolean;
    /** What the next question carries (shown as chips above the input). */
    attachments?: Attachment[];
    /** Index of the highlighted result, for screen readers. */
    activeIndex: number | null;
    oninput: (value: string) => void;
    onkeydown: (e: KeyboardEvent) => void;
    /** A chip's × was clicked. */
    onremove?: (index: number) => void;
    /** The mic button: start Windows voice typing in the box. */
    ondictate?: () => void;
  }
  let {
    value,
    chat = false,
    attachments = [],
    activeIndex,
    oninput,
    onkeydown,
    onremove,
    ondictate,
  }: Props = $props();
  let input: HTMLInputElement;

  const composing = $derived(attachments.length > 0);
  const placeholder = $derived(
    chat
      ? "Ask a follow-up…"
      : composing
        ? "Ask about this… (Enter asks, Esc clears)"
        : "Search apps and files, or ask Claude…",
  );

  /** Called when the launcher appears: typing replaces the old query. */
  export function focusAndSelect(): void {
    input.focus();
    input.select();
  }

  /** Puts the caret back in the box (after a click elsewhere in the bar). */
  export function focus(): void {
    input.focus();
  }

  $effect(() => input.focus());

  function chipGlyph(a: Attachment): string {
    if (a.type === "path") return GLYPHS[a.folder ? "folder" : "file"] ?? "";
    return GLYPHS.snippet ?? "";
  }

  function remove(index: number) {
    onremove?.(index);
    input.focus();
  }

  function dictate() {
    input.focus();
    ondictate?.();
  }
</script>

<div class="bar">
  {#if composing}
    <ul class="chips" aria-label="Attached to the question">
      {#each attachments as a, i (i)}
        <li class="chip" title={a.type === "path" ? a.path : chipLabel(a)}>
          {#if a.type === "image"}
            <img src={convertFileSrc(pictureKey(a.id), "icon")} alt="" draggable="false" />
          {:else}
            <!-- Static markup from glyphs.ts. -->
            <svg viewBox="0 0 24 24" aria-hidden="true">{@html chipGlyph(a)}</svg>
          {/if}
          <span class="label">{chipLabel(a)}</span>
          <button
            type="button"
            aria-label={`Remove ${chipName(a)}`}
            onmousedown={(e) => e.preventDefault()}
            onclick={() => remove(i)}>×</button
          >
        </li>
      {/each}
    </ul>
  {/if}
  <div class="row">
    <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
      {#if chat || composing}
        <!-- A four-point sparkle: you're talking to the AI. -->
        <path d="M12 3 C12.8 8.5 15.5 11.2 21 12 C15.5 12.8 12.8 15.5 12 21 C11.2 15.5 8.5 12.8 3 12 C8.5 11.2 11.2 8.5 12 3 Z" />
      {:else}
        <circle cx="10.5" cy="10.5" r="6.5" />
        <line x1="15.5" y1="15.5" x2="21" y2="21" />
      {/if}
    </svg>
    <input
      bind:this={input}
      {value}
      oninput={(e) => oninput(e.currentTarget.value)}
      {onkeydown}
      {placeholder}
      spellcheck={chat || composing}
      autocomplete="off"
      aria-label={chat ? "Message the AI" : composing ? "Question about the attachments" : "Search"}
      role={chat || composing ? undefined : "combobox"}
      aria-expanded={chat || composing ? undefined : activeIndex !== null}
      aria-controls={chat || composing ? undefined : "results"}
      aria-activedescendant={chat || composing || activeIndex === null ? undefined : `result-${activeIndex}`}
    />
    {#if ondictate}
      <!-- Keeps the caret in the box: voice typing writes where it is. -->
      <button
        type="button"
        class="mic"
        aria-label="Dictate (Windows voice typing, Win+H)"
        title="Dictate (Win+H)"
        onmousedown={(e) => e.preventDefault()}
        onclick={dictate}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">{@html GLYPHS.mic ?? ""}</svg>
      </button>
    {/if}
  </div>
</div>

<style>
  .bar {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4) var(--space-5);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .icon {
    width: 22px;
    height: 22px;
    flex: none;
    fill: none;
    stroke: var(--text-muted);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--font-size-input);
  }
  input::placeholder {
    color: var(--text-faint);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .chip {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    max-width: 100%;
    height: var(--chip-height);
    padding: 0 var(--space-1) 0 var(--space-2);
    border: 1px solid var(--approval-border);
    border-radius: var(--radius-keycap);
    background: var(--approval);
    font-size: var(--font-size-sub);
    color: var(--text);
  }
  .chip svg,
  .chip img {
    flex: none;
    width: var(--chip-icon);
    height: var(--chip-icon);
  }
  .chip svg {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .chip img {
    object-fit: cover;
    border-radius: var(--radius-icon);
  }
  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip button,
  .mic {
    flex: none;
    border: none;
    background: none;
    color: var(--text-muted);
    font: inherit;
  }
  .chip button {
    padding: 0 var(--space-1);
    font-size: var(--font-size-title);
    line-height: 1;
  }
  .mic {
    display: flex;
    padding: var(--space-1);
    border-radius: var(--radius-keycap);
  }
  .mic svg {
    width: 20px;
    height: 20px;
    fill: none;
    stroke: var(--text-faint);
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .mic:hover svg {
    stroke: var(--accent);
  }
  .chip button:focus-visible,
  .mic:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
