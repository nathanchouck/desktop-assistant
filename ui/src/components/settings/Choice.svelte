<script lang="ts">
  interface Props {
    /** [value, text] pairs. */
    options: [string, string][];
    value: string;
    /** For screen readers (the row's label). */
    label: string;
    onchange: (value: string) => void;
  }
  let { options, value, label, onchange }: Props = $props();
</script>

<div role="radiogroup" aria-label={label}>
  {#each options as [key, text] (key)}
    <button type="button" role="radio" aria-checked={value === key} class:on={value === key} onclick={() => onchange(key)}>
      {text}
    </button>
  {/each}
</div>

<style>
  div {
    display: flex;
    flex: none;
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    overflow: hidden;
  }
  button {
    padding: var(--space-1) var(--space-3);
    border: none;
    background: transparent;
    color: var(--text-muted);
    font: inherit;
    font-size: var(--font-size-sub);
  }
  button.on {
    background: var(--accent);
    color: var(--on-accent);
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
</style>
