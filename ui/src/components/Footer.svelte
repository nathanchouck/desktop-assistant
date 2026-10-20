<script lang="ts">
  interface Props {
    /** Shown instead of the key hints when set (errors, "coming soon"). */
    notice: string | null;
    /** The selected result can run as administrator. */
    canElevate: boolean;
  }
  let { notice, canElevate }: Props = $props();

  const hints = $derived<[string, string][]>([
    ["↵", "Open"],
    ...(canElevate ? ([["Ctrl ⇧ ↵", "Admin"]] as [string, string][]) : []),
    ["Esc", "Clear"],
  ]);
</script>

<footer>
  {#if notice}
    <span class="notice" role="status">{notice}</span>
  {:else}
    <span></span>
  {/if}
  <span class="hints">
    {#each hints as [key, label] (key)}
      <span class="hint"><kbd>{key}</kbd>{label}</span>
    {/each}
  </span>
</footer>

<style>
  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    height: var(--footer-height);
    padding: 0 var(--space-4);
    border-top: 1px solid var(--hairline);
    font-size: var(--font-size-sub);
    color: var(--text-faint);
  }
  .notice {
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hints {
    display: flex;
    flex: none;
    gap: var(--space-3);
  }
  .hint {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  kbd {
    padding: 0 var(--space-1);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    font: inherit;
    color: var(--text-muted);
  }
</style>
