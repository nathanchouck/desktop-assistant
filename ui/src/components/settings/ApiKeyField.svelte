<script lang="ts">
  // One API key: whether it's saved (and its last four characters), a
  // password field to save a new one, and Remove. The key itself never
  // comes back from the backend.
  import { commands, type SecretStatus } from "../../lib/bindings";
  import { savedLabel, type ApiKeyInfo } from "../../lib/mcp";
  import Button from "./Button.svelte";
  import Row from "./Row.svelte";

  interface Props {
    info: ApiKeyInfo;
    status: SecretStatus | undefined;
    /** The key was saved or removed. */
    onchange: (status: SecretStatus) => void;
  }
  let { info, status, onchange }: Props = $props();
  let draft = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);

  async function save() {
    if (!draft.trim() || busy) return;
    busy = true;
    const res = await commands.secretSet(info.name, draft);
    busy = false;
    if (res.status === "error") {
      error = res.error.message;
      return;
    }
    draft = "";
    error = null;
    onchange(res.data);
  }

  async function remove() {
    busy = true;
    const res = await commands.secretDelete(info.name);
    busy = false;
    if (res.status === "error") {
      error = res.error.message;
      return;
    }
    error = null;
    onchange(res.data);
  }
</script>

<Row label={info.label} hint={info.hint} stacked>
  <div class="key">
    <span class="state" class:saved={status?.saved}>{savedLabel(status)}</span>
    <input
      type="password"
      bind:value={draft}
      placeholder={status?.saved ? "Paste a new key to replace it" : info.placeholder}
      aria-label={info.label}
      autocomplete="off"
      spellcheck="false"
      onkeydown={(e) => {
        if (e.key === "Enter") void save();
      }}
    />
    <Button primary disabled={!draft.trim() || busy} onclick={() => void save()}>Save</Button>
    {#if status?.saved}
      <Button disabled={busy} label={`Remove the ${info.label}`} onclick={() => void remove()}>Remove</Button>
    {/if}
  </div>
  {#if error}
    <small class="error" role="alert">{error}</small>
  {/if}
</Row>

<style>
  .key {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .state {
    flex: none;
    min-width: 9em;
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
  .state.saved {
    color: var(--success);
  }
  input {
    flex: 1;
    min-width: 0;
    padding: 2px var(--space-2);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: transparent;
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--font-size-sub);
  }
  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .error {
    font-size: var(--font-size-sub);
    color: var(--danger);
  }
</style>
