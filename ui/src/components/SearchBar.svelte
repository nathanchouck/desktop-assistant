<script lang="ts">
  interface Props {
    value: string;
    /** Chatting: the box takes a follow-up instead of a search. */
    chat?: boolean;
    /** Index of the highlighted result, for screen readers. */
    activeIndex: number | null;
    oninput: (value: string) => void;
    onkeydown: (e: KeyboardEvent) => void;
  }
  let { value, chat = false, activeIndex, oninput, onkeydown }: Props = $props();
  let input: HTMLInputElement;

  /** Called when the launcher appears: typing replaces the old query. */
  export function focusAndSelect(): void {
    input.focus();
    input.select();
  }

  /** Back from a popup (the action menu): carry on typing where you were. */
  export function focus(): void {
    input.focus();
  }

  $effect(() => input.focus());
</script>

<div class="bar">
  <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
    {#if chat}
      <!-- A four-point sparkle: you're talking to Claude. -->
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
    placeholder={chat ? "Ask a follow-up…" : "Search apps and files, or ask Claude…"}
    spellcheck={chat}
    autocomplete="off"
    aria-label={chat ? "Message Claude" : "Search"}
    role={chat ? undefined : "combobox"}
    aria-expanded={chat ? undefined : activeIndex !== null}
    aria-controls={chat ? undefined : "results"}
    aria-activedescendant={chat || activeIndex === null ? undefined : `result-${activeIndex}`}
  />
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5);
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
</style>
