<script lang="ts">
  // API keys (kept in Windows Credential Manager) and the MCP registry:
  // servers added once, given to both Claude and ChatGPT.
  import { commands, type McpServer, type SecretStatus, type Settings } from "../../lib/bindings";
  import {
    API_KEYS,
    byName,
    describe,
    draftFrom,
    missingSecrets,
    newDraft,
    secretNames,
    type ServerDraft,
  } from "../../lib/mcp";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import ApiKeyField from "./ApiKeyField.svelte";
  import Button from "./Button.svelte";
  import McpServerEditor from "./McpServerEditor.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();

  /** What's saved in Credential Manager, by name (never the values). */
  let statuses = $state<Map<string, SecretStatus>>(new Map());
  /** The server being edited or added; `key` gives each a fresh editor. */
  let editing = $state<{ key: number; draft: ServerDraft } | null>(null);
  let opened = 0;
  /** The server whose Remove was pressed once. */
  let confirming = $state<string | null>(null);
  let error = $state<string | null>(null);

  async function refreshStatuses(servers: McpServer[]) {
    const names = [...API_KEYS.map((k) => k.name), ...servers.flatMap(secretNames)];
    const res = await commands.secretStatus(names);
    if (res.status === "ok") statuses = byName(res.data);
  }

  // Asked again whenever the list changes (here, or synced from another PC).
  $effect(() => {
    void refreshStatuses(store.settings?.mcp.servers ?? []);
  });

  function keySaved(status: SecretStatus) {
    statuses = new Map(statuses).set(status.name, status);
  }

  function open(draft: ServerDraft) {
    confirming = null;
    error = null;
    editing = { key: ++opened, draft };
  }

  function saved(settings: Settings) {
    editing = null;
    store.changed(settings);
    void refreshStatuses(settings.mcp.servers);
  }

  async function remove(name: string) {
    if (confirming !== name) {
      confirming = name;
      return;
    }
    confirming = null;
    const res = await commands.mcpServerRemove(name);
    if (res.status === "error") {
      error = res.error.message;
      return;
    }
    error = null;
    if (editing?.draft.previous === name) editing = null;
    store.changed(res.data);
  }

  function setEnabled(name: string, on: boolean) {
    void store.save((s) => {
      const server = s.mcp.servers.find((x) => x.name === name);
      if (server) server.enabled = on;
    });
  }
</script>

{#if store.settings}
  {@const mcp = store.settings.mcp}
  <Section title="API keys">
    {#each API_KEYS as info (info.name)}
      <ApiKeyField {info} status={statuses.get(info.name)} onchange={keySaved} />
    {/each}
    <small class="note">Kept in Windows Credential Manager on this PC, never in the settings file.</small>
  </Section>

  <Section title="MCP servers">
    <Row
      label="Give the AIs these servers"
      hint="Add a server once and both Claude and ChatGPT get it in new chats. Each AI's own MCP servers and connectors still load too."
    >
      <Toggle on={mcp.enabled} label="Give the AIs these servers" onchange={(on) => void store.save((s) => (s.mcp.enabled = on))} />
    </Row>
    <!-- Keyed by position: a hand-edited file may repeat a name. -->
    {#each mcp.servers as server, i (i)}
      {@const missing = missingSecrets(server, statuses)}
      <div class="server" class:off={!mcp.enabled || !server.enabled}>
        <div class="info">
          <span class="name">{server.name}</span>
          <small class="mono">{describe(server)}</small>
          {#if missing.length > 0}
            <small class="warning">Not saved on this PC: {missing.join(", ")}. Edit to enter it.</small>
          {/if}
        </div>
        <Toggle on={server.enabled} label={`Use ${server.name}`} onchange={(on) => setEnabled(server.name, on)} />
        <Button onclick={() => open(draftFrom(server))}>Edit</Button>
        <Button primary={confirming === server.name} onclick={() => void remove(server.name)}>
          {confirming === server.name ? "Remove?" : "Remove"}
        </Button>
      </div>
      {#if editing && editing.draft.previous === server.name}
        {#key editing.key}
          <McpServerEditor initial={editing.draft} {statuses} onsaved={saved} oncancel={() => (editing = null)} />
        {/key}
      {/if}
    {/each}
    {#if editing && editing.draft.previous === null}
      {#key editing.key}
        <McpServerEditor initial={editing.draft} {statuses} onsaved={saved} oncancel={() => (editing = null)} />
      {/key}
    {:else}
      <div class="add">
        <Button onclick={() => open(newDraft())}>Add server</Button>
      </div>
    {/if}
    {#if error}
      <small class="warning" role="alert">{error}</small>
    {/if}
  </Section>
{/if}

<style>
  .note {
    display: block;
    padding: var(--space-2) 0;
    font-size: var(--font-size-sub);
    color: var(--text-faint);
  }
  .server {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--hairline);
  }
  .server.off .info {
    opacity: 0.6;
  }
  .info {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }
  .info small {
    overflow: hidden;
    font-size: var(--font-size-sub);
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .info .warning,
  .warning {
    font-size: var(--font-size-sub);
    color: var(--danger);
    white-space: normal;
  }
  .add {
    padding: var(--space-2) 0;
  }
</style>
