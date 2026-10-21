<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
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
    margin-top: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--approval-border);
    border-radius: var(--radius-row);
    background: var(--approval);
    font-size: var(--font-size-sub);
  }
</style>
