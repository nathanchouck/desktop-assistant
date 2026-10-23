<script lang="ts">
  // Adds or changes one MCP server. Secret values are typed here and go
  // straight to Credential Manager on Save; a saved one is never shown.
  import { untrack } from "svelte";
  import { commands, type SecretStatus, type Settings } from "../../lib/bindings";
  import { editFrom, kindOf, valuePlaceholder, type ServerDraft } from "../../lib/mcp";
  import Button from "./Button.svelte";
  import Choice from "./Choice.svelte";

  interface Props {
    /** The server as the editor opens (a fresh draft for a new one). */
    initial: ServerDraft;
    statuses: Map<string, SecretStatus>;
    /** Saved: the settings now in use. */
    onsaved: (settings: Settings) => void;
    oncancel: () => void;
  }
  let { initial, statuses, onsaved, oncancel }: Props = $props();
  // The parent opens a new editor for each server, so this copy is the
  // editor's own from then on.
  let draft = $state(untrack(() => $state.snapshot(initial) as ServerDraft));
  let error = $state<string | null>(null);
  let busy = $state(false);

  const kind = $derived(kindOf(draft.transport));
  const values = $derived(draft.transport === "stdio" ? draft.env : draft.headers);
  const what = $derived(draft.transport === "stdio" ? "variable" : "header");

  function addValue() {
    values.push({ name: "", value: "", secret: draft.transport === "http", original: null });
  }

  async function save() {
    if (busy) return;
    busy = true;
    const res = await commands.mcpServerSave(editFrom(draft));
    busy = false;
    if (res.status === "error") {
      error = res.error.message;
      return;
    }
    onsaved(res.data);
  }

  function onkeydown(e: KeyboardEvent) {
    // Esc closes the editor, not the whole menu.
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      oncancel();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="editor" role="group" aria-label={initial.previous ? `Edit ${initial.previous}` : "New MCP server"} {onkeydown}>
  <div class="line">
    <label class="field grow">
      <span>Name</span>
      <input bind:value={draft.name} placeholder="github" class="mono" spellcheck="false" />
    </label>
    <div class="field">
      <span>Runs as</span>
      <Choice
        options={[
          ["stdio", "Command"],
          ["http", "URL"],
        ]}
        value={draft.transport}
        label="Runs as"
        onchange={(v) => (draft.transport = v as "stdio" | "http")}
      />
    </div>
  </div>

  {#if draft.transport === "stdio"}
    <label class="field">
      <span>Command</span>
      <input bind:value={draft.command} placeholder="npx, uvx, or a full path" class="mono" spellcheck="false" />
    </label>
    <small class="hint">On Windows, a server started with npx needs the command <code>cmd</code> and the arguments <code>/c</code>, <code>npx</code>, …</small>
    <label class="field">
      <span>Arguments, one per line</span>
      <textarea bind:value={draft.args} placeholder={"-y\n@modelcontextprotocol/server-github"} class="mono" rows="3" spellcheck="false"></textarea>
    </label>
  {:else}
    <label class="field">
      <span>URL</span>
      <input bind:value={draft.url} placeholder="https://example.com/mcp" class="mono" spellcheck="false" />
    </label>
  {/if}

  <div class="field">
    <span>{draft.transport === "stdio" ? "Environment variables" : "Headers"}</span>
    {#each values as value, i (i)}
      <div class="value">
        <input
          bind:value={value.name}
          placeholder={draft.transport === "stdio" ? "NAME" : "Authorization"}
          aria-label={`${what} name`}
          class="mono name"
          spellcheck="false"
        />
        <input
          bind:value={value.value}
          type={value.secret ? "password" : "text"}
          placeholder={valuePlaceholder(draft, kind, value, statuses)}
          aria-label={`${what} value`}
          autocomplete="off"
          class="mono"
          spellcheck="false"
        />
        <label class="secret">
          <input type="checkbox" bind:checked={value.secret} />
          Secret
        </label>
        <Button label={`Remove ${what} ${value.name}`} onclick={() => values.splice(i, 1)}>×</Button>
      </div>
    {/each}
    <div>
      <Button onclick={addValue}>Add {what}</Button>
    </div>
    <small class="hint">A secret is kept in Windows Credential Manager on this PC, never in the settings file. Leave a saved one empty to keep it.</small>
  </div>

  {#if error}
    <small class="error" role="alert">{error}</small>
  {/if}
  <div class="actions">
    <Button onclick={oncancel}>Cancel</Button>
    <Button primary disabled={busy} onclick={() => void save()}>Save</Button>
  </div>
</div>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: var(--space-2) 0;
    padding: var(--space-3);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-row);
  }
  .line {
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
  .grow {
    flex: 1;
  }
  .value {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .value input:not([type="checkbox"]) {
    flex: 2;
  }
  .value .name {
    flex: 1;
  }
  input:not([type="checkbox"]),
  textarea {
    min-width: 0;
    padding: 2px var(--space-2);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--font-size-sub);
  }
  textarea {
    resize: vertical;
  }
  .mono {
    font-family: var(--font-mono);
  }
  input:focus-visible,
  textarea:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .secret {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
  .secret input {
    margin: 0;
    accent-color: var(--accent);
  }
  .hint {
    font-size: var(--font-size-sub);
    color: var(--text-faint);
  }
  code {
    font-family: var(--font-mono);
  }
  .error {
    font-size: var(--font-size-sub);
    color: var(--danger);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
