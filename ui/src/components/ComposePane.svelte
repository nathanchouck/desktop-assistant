<script lang="ts">
  // What the next question carries, in full: the selected text, the file's
  // path, the snipped picture. The chips above the input are the short form.
  import { convertFileSrc } from "@tauri-apps/api/core";
  import type { Attachment } from "../lib/bindings";
  import { fileName, pictureKey, textPreview } from "../lib/compose";

  interface Props {
    attachments: Attachment[];
    /** Who Enter asks ("Claude"). */
    ai: string;
  }
  let { attachments, ai }: Props = $props();
</script>

<div class="pane">
  <p class="lead">Type your question about {attachments.length === 1 ? "this" : "these"}, then press Enter to ask {ai}.</p>
  {#each attachments as a, i (i)}
    {#if a.type === "text"}
      <section aria-label="Selected text">
        <h4>Selected text · {[...a.text].length} characters</h4>
        <pre>{textPreview(a.text)}</pre>
      </section>
    {:else if a.type === "path"}
      <section aria-label={a.folder ? "Folder" : "File"}>
        <h4>{a.folder ? "Folder" : "File"} · {fileName(a.path)}</h4>
        <p class="path">{a.path}</p>
      </section>
    {:else}
      <section aria-label="Picture">
        <h4>Picture · {a.width}×{a.height}</h4>
        <img src={convertFileSrc(pictureKey(a.id), "icon")} alt="What you attached" draggable="false" />
      </section>
    {/if}
  {/each}
</div>

<style>
  .pane {
    height: calc(var(--row-height) * var(--results-rows) + var(--space-2));
    padding: var(--space-2) var(--space-5) var(--space-4);
    overflow-y: auto;
    scrollbar-width: thin;
    font-size: var(--font-size-chat);
  }
  p,
  h4,
  pre {
    margin: 0;
  }
  .lead {
    margin-bottom: var(--space-3);
    color: var(--text-muted);
  }
  section {
    margin-bottom: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--approval-border);
    border-radius: var(--radius-row);
    background: var(--approval);
  }
  h4 {
    margin-bottom: var(--space-1);
    font-size: var(--font-size-sub);
    font-weight: 600;
    color: var(--text-muted);
  }
  pre,
  .path {
    font-family: var(--font-mono);
    font-size: var(--font-size-sub);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  img {
    display: block;
    max-width: 100%;
    max-height: var(--compose-preview-height);
    border-radius: var(--radius-icon);
  }
</style>
