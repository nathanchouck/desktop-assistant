<script lang="ts">
  import { commands } from "../lib/bindings";
  import { approvalSummary, isThinking, type Transcript } from "../lib/chat";
  import { renderMarkdown } from "../lib/markdown";

  interface Props {
    transcript: Transcript;
    onanswer: (allow: boolean) => void;
  }
  let { transcript, onanswer }: Props = $props();
  let pane: HTMLDivElement;
  /** Follow new text unless you've scrolled up to read. */
  let pinned = true;

  const LINE = 40;

  export function scroll(delta: number, pages: boolean): void {
    pane.scrollBy({ top: delta * (pages ? pane.clientHeight - LINE : LINE) });
  }

  const thinking = $derived(isThinking(transcript));

  // Re-runs on every streamed change (reads each entry), after the DOM updates.
  $effect(() => {
    void JSON.stringify(transcript.entries);
    void thinking;
    if (pinned) pane.scrollTop = pane.scrollHeight;
  });

  function onscroll() {
    pinned = pane.scrollHeight - pane.scrollTop - pane.clientHeight < LINE;
  }

  /** Links open in the browser; code blocks copy. Never navigate the launcher. */
  function onclick(e: MouseEvent) {
    const target = e.target as Element | null;
    const copy = target?.closest("button.copy");
    if (copy) {
      const code = copy.parentElement?.querySelector("code")?.textContent ?? "";
      void navigator.clipboard.writeText(code).then(() => {
        copy.textContent = "Copied";
        setTimeout(() => (copy.textContent = "Copy"), 1200);
      });
      return;
    }
    const link = target?.closest("a");
    if (link) {
      e.preventDefault();
      const href = link.getAttribute("href") ?? "";
      if (/^https?:\/\//i.test(href)) void commands.openExternal(href);
    }
  }
</script>

<!-- Keyboard scrolling is handled by the input (see App.svelte). -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="pane" bind:this={pane} {onscroll} {onclick} onauxclick={onclick} aria-live="polite">
  {#each transcript.entries as entry, i (i)}
    {#if entry.kind === "user"}
      <p class="user">{entry.text}</p>
      {#if entry.context}
        <p class="context" title="What the AI was told about your screen">Sees: {entry.context}</p>
      {/if}
    {:else if entry.kind === "assistant"}
      <div class="answer">{@html renderMarkdown(entry.text)}</div>
    {:else if entry.kind === "tool"}
      <p class="tool {entry.state}">
        <span class="status" aria-label={entry.state}></span>
        <span class="name">{entry.name}</span>
        {#if entry.summary}<span class="summary">{entry.summary}</span>{/if}
      </p>
    {:else if entry.kind === "approval"}
      {#if entry.state === "pending"}
        <section class="approval" aria-label="Approval">
          <p>Allow <strong>{entry.tool}</strong>{approvalSummary(entry) ? ` · ${approvalSummary(entry)}` : ""}?</p>
          {#if entry.detail}
            <pre class="detail">{entry.detail}</pre>
          {/if}
          <div class="buttons">
            <button type="button" class="approve" onclick={() => onanswer(true)}>
              Approve <kbd>Ctrl Y</kbd>
            </button>
            <button type="button" onclick={() => onanswer(false)}>Deny <kbd>Ctrl N</kbd></button>
          </div>
        </section>
      {:else}
        <p class="verdict {entry.state}">{entry.state === "allowed" ? "Approved" : "Denied"} · {entry.tool}</p>
      {/if}
    {:else if entry.kind === "error"}
      <p class="error" role="alert">{entry.text}</p>
    {:else if entry.kind === "note"}
      <p class="note">{entry.text}</p>
    {/if}
  {/each}
  {#if thinking}
    <p class="thinking" aria-label="Thinking"><span></span><span></span><span></span></p>
  {/if}
</div>

<style>
  .pane {
    height: calc(var(--row-height) * var(--results-rows) + var(--space-2));
    padding: var(--space-2) var(--space-5) var(--space-4);
    overflow-y: auto;
    scrollbar-width: thin;
    font-size: var(--font-size-chat);
    line-height: 1.5;
    user-select: text;
  }
  p {
    margin: 0;
  }
  .user {
    width: fit-content;
    max-width: 85%;
    margin: var(--space-2) 0 var(--space-2) auto;
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--selection-border);
    border-radius: var(--radius-row);
    background: var(--selection-sheen), var(--selection);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .context {
    margin: calc(-1 * var(--space-1)) 0 var(--space-2);
    overflow: hidden;
    text-align: right;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--font-size-sub);
    color: var(--text-faint);
  }
  .answer {
    margin: var(--space-2) 0;
    overflow-wrap: anywhere;
  }
  .answer :global(p),
  .answer :global(ul),
  .answer :global(ol),
  .answer :global(pre),
  .answer :global(table),
  .answer :global(blockquote) {
    margin: 0 0 var(--space-2);
  }
  .answer :global(ul),
  .answer :global(ol) {
    padding-left: var(--space-5);
  }
  .answer :global(h1),
  .answer :global(h2),
  .answer :global(h3),
  .answer :global(h4) {
    margin: var(--space-3) 0 var(--space-1);
    font-size: var(--font-size-title);
  }
  .answer :global(a) {
    color: var(--accent);
    text-decoration: none;
  }
  .answer :global(a:hover) {
    text-decoration: underline;
  }
  .answer :global(code) {
    padding: 0 3px;
    border-radius: var(--radius-keycap);
    background: var(--code);
    font-family: var(--font-mono);
    font-size: 0.92em;
  }
  /* LaTeX: a wide equation scrolls sideways instead of widening the pane. */
  .answer :global(.math) {
    margin: var(--space-2) 0;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: thin;
  }
  .answer :global(.katex) {
    font-size: 1.1em;
  }
  .answer :global(.katex-display) {
    margin: 0;
  }
  .answer :global(pre) {
    position: relative;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-keycap);
    background: var(--code);
    overflow-x: auto;
  }
  .answer :global(pre code) {
    padding: 0;
    background: none;
  }
  .answer :global(pre .copy) {
    position: absolute;
    top: var(--space-1);
    right: var(--space-1);
    padding: 0 var(--space-2);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    color: var(--text-muted);
    font: inherit;
    font-size: var(--font-size-sub);
    opacity: 0;
    transition: opacity var(--duration-fast) var(--ease);
  }
  .answer :global(pre:hover .copy) {
    opacity: 1;
  }
  .answer :global(table) {
    border-collapse: collapse;
  }
  .answer :global(th),
  .answer :global(td) {
    padding: 2px var(--space-2);
    border: 1px solid var(--hairline);
  }
  .answer :global(blockquote) {
    padding-left: var(--space-3);
    border-left: 3px solid var(--hairline);
    color: var(--text-muted);
  }
  .answer :global(> :last-child) {
    margin-bottom: 0;
  }

  .tool {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-1) 0;
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
  .tool .name {
    flex: none;
    font-weight: 600;
  }
  .tool .summary {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    color: var(--text-faint);
  }
  .status {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--success);
  }
  .tool.running .status {
    background: var(--accent);
    animation: pulse 1s var(--ease) infinite alternate;
  }
  .tool.failed .status {
    background: var(--danger);
  }

  .approval {
    margin: var(--space-2) 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--approval-border);
    border-radius: var(--radius-row);
    background: var(--approval);
    font-size: var(--font-size-sub);
  }
  .detail {
    max-height: 120px;
    margin: var(--space-2) 0 0;
    padding: var(--space-2);
    overflow: auto;
    border-radius: var(--radius-keycap);
    background: var(--code);
    font-family: var(--font-mono);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .verdict {
    margin: var(--space-1) 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--font-size-sub);
    color: var(--text-faint);
  }
  .verdict.denied {
    color: var(--danger);
  }
  .buttons {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    color: var(--text);
    font: inherit;
  }
  button.approve {
    border-color: transparent;
    background: var(--accent);
    color: var(--on-accent);
  }
  kbd {
    font: inherit;
    opacity: 0.7;
  }

  .error {
    margin: var(--space-2) 0;
    font-size: var(--font-size-sub);
    color: var(--danger);
    white-space: pre-wrap;
  }

  .note {
    margin: var(--space-1) 0;
    font-size: var(--font-size-sub);
    color: var(--text-faint);
  }

  .thinking {
    display: flex;
    gap: var(--space-1);
    margin: var(--space-3) 0;
  }
  .thinking span {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-faint);
    animation: pulse 0.9s var(--ease) infinite alternate;
  }
  .thinking span:nth-child(2) {
    animation-delay: 0.15s;
  }
  .thinking span:nth-child(3) {
    animation-delay: 0.3s;
  }
  @media (prefers-reduced-motion: reduce) {
    .tool.running .status,
    .thinking span {
      animation: none;
    }
  }
  @keyframes pulse {
    from {
      opacity: 0.25;
    }
    to {
      opacity: 1;
    }
  }
</style>
