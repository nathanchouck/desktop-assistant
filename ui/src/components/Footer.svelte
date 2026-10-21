<script lang="ts">
  import { commands, type Toast } from "../lib/bindings";
  import type { Hint } from "../lib/keys";

  interface Props {
    /** Left side: an error or status line, or what's answering. */
    notice: string | null;
    /** Right side: key hints. */
    hints: Hint[];
    /** A message from the backend; shown instead of the notice for a while. */
    toast?: Toast | null;
    ondismiss?: () => void;
  }
  let { notice, hints, toast = null, ondismiss }: Props = $props();
</script>

<footer>
  {#if toast}
    <span class="toast" class:error={toast.level === "error"} role="alert">
      <span class="message">{toast.message}</span>
      {#if toast.level === "error"}
        <button type="button" onclick={() => void commands.openLogFolder()}>Open log</button>
      {/if}
      <button type="button" class="close" aria-label="Dismiss" onclick={() => ondismiss?.()}>×</button>
    </span>
  {:else if notice}
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
  .notice,
  .message {
    min-width: 0;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .toast {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .toast.error .message {
    color: var(--danger);
  }
  .toast button {
    flex: none;
    padding: 0 var(--space-2);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    color: var(--text-muted);
    font: inherit;
  }
  .toast .close {
    border: none;
    background: none;
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
    white-space: nowrap;
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
