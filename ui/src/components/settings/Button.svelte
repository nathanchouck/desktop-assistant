<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    onclick: () => void;
    primary?: boolean;
    disabled?: boolean;
    /** For icon-only buttons. */
    label?: string;
    children: Snippet;
  }
  let { onclick, primary = false, disabled = false, label, children }: Props = $props();
</script>

<button type="button" class:primary {disabled} aria-label={label} {onclick}>{@render children()}</button>

<style>
  button {
    flex: none;
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    color: var(--text);
    font: inherit;
    font-size: var(--font-size-sub);
  }
  button:disabled {
    opacity: 0.4;
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  button.primary {
    border-color: transparent;
    background: var(--accent);
    color: var(--on-accent);
  }
</style>
