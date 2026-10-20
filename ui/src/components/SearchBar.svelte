<script lang="ts">
  interface Props {
    value: string;
    /** Index of the highlighted result, for screen readers. */
    activeIndex: number | null;
    oninput: (value: string) => void;
    onkeydown: (e: KeyboardEvent) => void;
  }
  let { value, activeIndex, oninput, onkeydown }: Props = $props();
  let input: HTMLInputElement;

  $effect(() => input.focus());
</script>

<div class="bar">
  <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
    <circle cx="10.5" cy="10.5" r="6.5" />
    <line x1="15.5" y1="15.5" x2="21" y2="21" />
  </svg>
  <input
    bind:this={input}
    {value}
    oninput={(e) => oninput(e.currentTarget.value)}
    {onkeydown}
    placeholder="Search apps and files, or ask Claude…"
    spellcheck="false"
    autocomplete="off"
    aria-label="Search"
    role="combobox"
    aria-expanded={activeIndex !== null}
    aria-controls="results"
    aria-activedescendant={activeIndex === null ? undefined : `result-${activeIndex}`}
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
