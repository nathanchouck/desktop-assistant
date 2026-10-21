<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    /** The status text and any button next to it. */
    children: Snippet;
  }
  let { children }: Props = $props();
</script>

<div>{@render children()}</div>

<style>
  div {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-1) 0 var(--space-2);
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
</style>
