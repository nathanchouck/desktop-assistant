<script lang="ts">
  import { answeringLabel, PROVIDER_NAMES } from "../lib/chat";
  import { sideStatus, STATUS_LABELS } from "../lib/compare";
  import { GLYPHS } from "../lib/glyphs";
  import type { CompareStore } from "../lib/stores/compare.svelte";
  import ChatPane from "./ChatPane.svelte";

  interface Props {
    store: CompareStore;
    /** A column's own Approve / Deny button was clicked. */
    onanswer: (side: number, allow: boolean) => void;
  }
  let { store, onanswer }: Props = $props();
  let panes = $state<(ChatPane | undefined)[]>([]);

  /** Scrolls both columns together (the keys are the chat's). */
  export function scroll(delta: number, pages: boolean): void {
    for (const pane of panes) pane?.scroll(delta, pages);
  }
</script>

<div class="compare">
  {#each store.sides as side, i (side.provider)}
    <section class="column" aria-label={PROVIDER_NAMES[side.provider]}>
      <header>
        <svg class="ai" viewBox="0 0 24 24" aria-hidden="true">
          <!-- Static markup from glyphs.ts. -->
          {@html GLYPHS[side.provider === "claude" ? "sparkle" : "chatgpt"] ?? ""}
        </svg>
        <span class="name">{answeringLabel(side.provider, side.transcript.model)}</span>
        <span class="status">{STATUS_LABELS[sideStatus(side.transcript)]}</span>
        <kbd title={`Carry on with ${PROVIDER_NAMES[side.provider]} only`}>Ctrl {i + 1}</kbd>
      </header>
      <ChatPane
        bind:this={panes[i]}
        transcript={side.transcript}
        who={PROVIDER_NAMES[side.provider]}
        keys={store.pending?.side === i}
        onanswer={(allow) => onanswer(i, allow)}
      />
    </section>
  {/each}
</div>

<style>
  .compare {
    display: grid;
    grid-template-columns: 1fr 1fr;
  }
  .column {
    min-width: 0;
    /* The chat below the title bar fills the usual results height. */
    --chat-pane-height: calc(
      var(--row-height) * var(--results-rows) + var(--space-2) - var(--compare-head-height)
    );
    --chat-pane-gutter: var(--space-4);
  }
  .column + .column {
    border-left: 1px solid var(--hairline);
  }
  header {
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--compare-head-height);
    padding: 0 var(--space-4);
    border-bottom: 1px solid var(--hairline);
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
  .ai {
    flex: none;
    width: 16px;
    height: 16px;
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
    color: var(--text);
  }
  .status {
    flex: 1;
    white-space: nowrap;
    color: var(--text-faint);
  }
  kbd {
    flex: none;
    padding: 0 var(--space-1);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    font: inherit;
    color: var(--text-muted);
  }
</style>
