<script lang="ts">
  import { commands } from "../../lib/bindings";
  import { shortcutFromEvent } from "../../lib/keys";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import { timeoutRow, winKeyStatusText } from "../../lib/winKey";
  import Button from "./Button.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();
  let recording = $state(false);

  async function startRecording() {
    recording = true;
    // Otherwise pressing the current shortcut would hide the launcher.
    await commands.shortcutPause(true);
  }

  async function stopRecording(shortcut: string | null) {
    recording = false;
    await commands.shortcutPause(false);
    if (shortcut) await store.save((s) => (s.hotkeys.toggle = shortcut));
  }

  function onRecorderKey(e: KeyboardEvent) {
    if (!recording) return;
    // Every key belongs to the recorder now (Esc cancels, not closes).
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") void stopRecording(null);
    else {
      const shortcut = shortcutFromEvent(e);
      if (shortcut) void stopRecording(shortcut);
    }
  }

  async function setHookTimeout(longer: boolean) {
    const res = await commands.winKeySetTimeout(longer);
    store.error = res.status === "error" ? res.error.message : null;
    await store.refreshStatus();
  }
</script>

{#if store.settings}
  {@const s = store.settings}
  {@const status = store.status}
  <Section title="Opening the launcher">
    <Row label="Shortcut" hint="Click, then press the keys you want.">
      <button
        type="button"
        class="field"
        class:recording
        onclick={() => (recording ? stopRecording(null) : startRecording())}
        onkeydown={onRecorderKey}
        onblur={() => recording && stopRecording(null)}
      >
        {recording ? "Press keys… (Esc to cancel)" : s.hotkeys.toggle}
      </button>
    </Row>
    <Row label="Tap the Windows key">
      {#snippet hintContent()}
        Opens the launcher instead of Start.
        {#if status}
          {winKeyStatusText(status.win_key)}
        {/if}
      {/snippet}
      <Toggle
        on={s.hotkeys.win_key_override}
        label="Tap the Windows key"
        onchange={(on) => void store.save((n) => (n.hotkeys.win_key_override = on))}
      />
    </Row>
    {#if s.hotkeys.win_key_override}
      <Row
        label="Close Start if it opens anyway"
        hint="When Windows acts on a tap before the launcher can hide it, close Start and bring the launcher back."
      >
        <Toggle
          on={s.hotkeys.close_start}
          label="Close Start if it opens anyway"
          onchange={(on) => void store.save((n) => (n.hotkeys.close_start = on))}
        />
      </Row>
      {#if status?.win_key.running}
        {@const row = timeoutRow(status.win_key)}
        <Row label="Give the Windows key more time" hint={row.hint}>
          <Button onclick={() => void setHookTimeout(row.longer)}>{row.button}</Button>
        </Row>
      {/if}
    {/if}
    <Row label="Start with Windows">
      <Toggle
        on={s.general.launch_at_login}
        label="Start with Windows"
        onchange={(on) => void store.save((n) => (n.general.launch_at_login = on))}
      />
    </Row>
  </Section>
{/if}

<style>
  .field {
    flex: none;
    min-width: 120px;
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--font-size-sub);
  }
  .field.recording {
    border-color: var(--accent);
    color: var(--accent);
  }
  .field:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
