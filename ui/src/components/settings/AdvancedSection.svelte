<script lang="ts">
  import { onMount } from "svelte";
  import { commands, type FolderCheck, type SettingsFolder } from "../../lib/bindings";
  import { folderHint, moveChoices, movedMessage } from "../../lib/settingsFolder";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Banner from "./Banner.svelte";
  import Button from "./Button.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Status from "./Status.svelte";
  import TextField from "./TextField.svelte";

  let { store }: { store: SettingsStore } = $props();
  /** Where settings are kept. */
  let folder = $state<SettingsFolder | null>(null);
  /** A folder typed in, checked and waiting for a choice. */
  let typed = $state<FolderCheck | null>(null);
  let notice = $state<string | null>(null);
  let problem = $state<string | null>(null);
  let busy = $state(false);

  onMount(() => void load());

  async function load() {
    const res = await commands.settingsFolderGet();
    if (res.status === "ok") folder = res.data;
    else problem = res.error.message;
  }

  async function check(text: string) {
    typed = null;
    notice = null;
    problem = null;
    if (!text.trim()) return;
    const res = await commands.settingsFolderCheck(text);
    if (res.status === "ok") typed = res.data;
    else problem = res.error.message;
  }

  async function moveTo(target: string, replace: boolean) {
    if (!folder || busy) return;
    const before = folder;
    busy = true;
    const res = await commands.settingsFolderMove(target, replace);
    busy = false;
    finish(res, before);
  }

  async function reset() {
    if (!folder || busy) return;
    const before = folder;
    busy = true;
    const res = await commands.settingsFolderReset();
    busy = false;
    finish(res, before);
  }

  function finish(res: Awaited<ReturnType<typeof commands.settingsFolderReset>>, before: SettingsFolder) {
    if (res.status === "error") {
      problem = res.error.message;
      notice = null;
      return;
    }
    folder = res.data.location;
    typed = null;
    problem = null;
    notice = movedMessage(res.data, before);
  }

  async function openLocal() {
    const res = await commands.settingsLocalOpen();
    if (res.status === "error") problem = res.error.message;
  }
</script>

{#snippet choices(target: FolderCheck)}
  <div class="choices">
    {#each moveChoices(target) as choice (choice.label)}
      <Button primary={choice.primary} disabled={busy} onclick={() => void moveTo(target.folder, choice.replace)}>
        {choice.label}
      </Button>
    {/each}
  </div>
{/snippet}

{#if store.settings}
  <Section title="Advanced">
    {#if folder}
      <Row label="Settings folder" stacked hint={folderHint(folder)}>
        <div class="current">
          <code>{folder.folder}</code>
          {#if !folder.is_default}
            <Button disabled={busy} onclick={() => void reset()}>Back to default</Button>
          {/if}
        </div>
        {#if folder.missing}
          <Banner>
            <span>{folder.missing} isn't there right now, so settings come from the default folder until it is.</span>
          </Banner>
        {/if}
        {#if folder.unreadable}
          <Banner>
            <span>settings.toml can't be read ({folder.unreadable}). Only this PC's settings are saved until it can.</span>
          </Banner>
        {/if}
        {#if folder.suggestion}
          <Banner>
            <span>
              {folder.suggestion.has_settings ? "Another PC keeps its settings in OneDrive:" : "Share settings through OneDrive:"}
              <code>{folder.suggestion.folder}</code>
            </span>
            {@render choices(folder.suggestion)}
          </Banner>
        {/if}
        <div class="other">
          <TextField
            value=""
            placeholder={String.raw`Another folder, like D:\Sync\desktop-assistant`}
            label="Another settings folder"
            mono
            onchange={(text) => void check(text)}
          />
        </div>
        {#if typed}
          <Banner>
            <span>
              {typed.has_settings ? "That folder has settings already:" : "Keep settings in"}
              <code>{typed.folder}</code>
            </span>
            {@render choices(typed)}
          </Banner>
        {/if}
        {#if problem}
          <p class="problem" role="alert">{problem}</p>
        {:else if notice}
          <Status><span role="status">{notice}</span></Status>
        {/if}
      </Row>
      <Row
        label="This PC only"
        hint="Index folders, exclusions, the Claude Code and Codex locations, the scripts folder, start at login and the Windows key stay on this PC, so syncing never changes them. Any key you add to this file stays per PC too."
      >
        <Button onclick={() => void openLocal()}>Open</Button>
      </Row>
    {/if}
    <Row label="Settings file" hint="Everything here, as text, for anything this menu doesn't cover. Edits apply when you save.">
      <Button onclick={() => void commands.itemActivate("cmd:settings-file", "run")}>Open</Button>
    </Row>
    <Row label="Log files" hint="What the launcher did, for when something goes wrong.">
      <Button onclick={() => void commands.openLogFolder()}>Open</Button>
    </Row>
  </Section>
{/if}

<style>
  .current {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  code {
    min-width: 0;
    overflow-wrap: anywhere;
    font-family: var(--font-mono);
    font-size: var(--font-size-sub);
  }
  .choices {
    display: flex;
    flex: none;
    gap: var(--space-2);
  }
  .other {
    display: flex;
    flex-direction: column;
  }
  .problem {
    margin: 0;
    font-size: var(--font-size-sub);
    color: var(--danger);
  }
</style>
