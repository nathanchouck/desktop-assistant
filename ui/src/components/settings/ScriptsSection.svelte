<script lang="ts">
  // Script commands: the folder, the time limit, and what the folder holds
  // (with why a file isn't a command).
  import { commands, type ScriptsOverview } from "../../lib/bindings";
  import { TIMEOUT_MAX, TIMEOUT_MIN, scriptDetail, scriptUsage, scriptsSummary, stepTimeout } from "../../lib/scripts";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import ItemIcon from "../ItemIcon.svelte";
  import Button from "./Button.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Status from "./Status.svelte";
  import Stepper from "./Stepper.svelte";
  import TextField from "./TextField.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();
  let overview = $state<ScriptsOverview | null>(null);
  let loading = $state(false);

  /** What the folder holds; `reload` reads it again first. */
  async function load(reload: boolean) {
    loading = true;
    const res = await commands.scriptsList(reload);
    loading = false;
    if (res.status === "ok") overview = res.data;
    else store.error = res.error.message;
  }

  $effect(() => {
    void load(false);
  });

  async function save(change: Parameters<SettingsStore["save"]>[0]) {
    // The list follows the new folder (or shows nothing while off).
    if (await store.save(change)) await load(true);
  }

  async function openFolder() {
    const res = await commands.scriptsOpenFolder();
    if (res.status === "error") store.error = res.error.message;
    // It may have just been made, with the example in it.
    await load(true);
  }
</script>

{#if store.settings}
  {@const sc = store.settings.scripts}
  <Section title="Script commands">
    <Row label="Script commands" hint="Scripts in a folder become commands: type a script's title and press Enter.">
      <Toggle on={sc.enabled} label="Script commands" onchange={(on) => void save((s) => (s.scripts.enabled = on))} />
    </Row>
    {#if sc.enabled}
      <Row
        label="Folder"
        hint="PowerShell, Python, batch, Node.js and Git Bash scripts with a @title comment at the top. README.txt in the folder has the details. Empty: the default folder."
        stacked
      >
        <TextField
          value={sc.folder}
          placeholder={overview?.default_folder ?? ""}
          label="Scripts folder"
          mono
          onchange={(v) => void save((s) => (s.scripts.folder = v.trim()))}
        />
      </Row>
      <Row label="Time limit" hint="Seconds a script may run before it's stopped, with everything it started.">
        <Stepper
          value={sc.timeout_secs}
          min={TIMEOUT_MIN}
          max={TIMEOUT_MAX}
          onchange={(v) => void save((s) => (s.scripts.timeout_secs = stepTimeout(sc.timeout_secs, v > sc.timeout_secs)))}
        />
      </Row>
      <Status>
        <span>{overview ? scriptsSummary(overview) : "Reading the folder…"}</span>
        <span class="buttons">
          <Button onclick={() => void openFolder()}>Open folder</Button>
          <Button disabled={loading} onclick={() => void load(true)}>Reload</Button>
        </span>
      </Status>
      {#if overview}
        <ul class="scripts">
          {#each overview.scripts as script (script.file)}
            <li>
              <ItemIcon kind="script" icon={script.icon} />
              <span class="text">
                <span class="title">{script.title}<kbd>{scriptUsage(script)}</kbd></span>
                <small>{script.description ? `${script.description} · ` : ""}{scriptDetail(script)}</small>
              </span>
            </li>
          {/each}
          {#each overview.problems as problem (problem.file)}
            <li class="problem">
              <ItemIcon kind="script" icon="glyph:warning" />
              <span class="text">
                <span class="title mono">{problem.file}</span>
                <small>{problem.message}</small>
              </span>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </Section>
{/if}

<style>
  .buttons {
    display: flex;
    flex: none;
    gap: var(--space-2);
  }
  .scripts {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-1) 0;
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .title {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    color: var(--text);
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--font-size-sub);
  }
  kbd {
    padding: 0 var(--space-1);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    font-family: var(--font-mono);
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
  small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
  .problem small {
    color: var(--danger);
  }
</style>
