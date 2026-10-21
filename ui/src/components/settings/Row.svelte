<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    label: string;
    /** Smaller text under the label. */
    hint?: string;
    /** Control under the label instead of beside it (lists, editors). */
    stacked?: boolean;
    /** The control. */
    children?: Snippet;
    /** Rich hint (overrides `hint`). */
    hintContent?: Snippet;
  }
  let { label, hint, stacked = false, children, hintContent }: Props = $props();
</script>

<div class="row" class:stacked>
  <div class="label">
    <span>{label}</span>
    {#if hintContent}
      <small>{@render hintContent()}</small>
    {:else if hint}
      <small>{hint}</small>
    {/if}
  </div>
  {@render children?.()}
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--hairline);
  }
  .row.stacked {
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-2);
  }
  .label {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  small {
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
</style>
