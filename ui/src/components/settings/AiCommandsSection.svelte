<script lang="ts">
  // AI inputs: the "ask about the selection" shortcut, and saved AI
  // commands (name, keyword, AI, icon, prompt with placeholders).
  import { commands, type AiCommand, type CommandAi } from "../../lib/bindings";
  import { AI_CHOICES, commandWarnings, newCommand, PLACEHOLDERS } from "../../lib/aiCommands";
  import { shortcutFromEvent } from "../../lib/keys";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Button from "./Button.svelte";
  import Choice from "./Choice.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import TextField from "./TextField.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();
  let recording = $state(false);

  const PROMPT_HINT = `What's sent. Placeholders: ${PLACEHOLDERS.join(" ")}`;
  const LIST_HINT =
    "Find one by name or keyword; text typed after it fills {text}. Also lists Ask About Screen Region and Dictate.";
  const PLACEHOLDER_HINT =
    "{selection} copies what's selected in the window you came from, {clipboard} is the clipboard's text, " +
    "{text} what you type after the name. {text|clipboard} takes the first that isn't empty. " +
    "Icons: sparkle, calendar, mail, drive, chat, globe… or an emoji.";

  async function pauseShortcuts(paused: boolean) {
    // Neither shortcut may fire while you press the new keys.
    await commands.shortcutPause(paused);
    await commands.aiSelectionShortcutPause(paused);
  }

  async function startRecording() {
    recording = true;
    await pauseShortcuts(true);
  }

  async function stopRecording(shortcut: string | null) {
    recording = false;
    await pauseShortcuts(false);
    if (shortcut) await store.save((s) => (s.ai_commands.selection_shortcut = shortcut));
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

  function edit(index: number, change: (command: AiCommand) => void) {
    void store.save((s) => {
      const command = s.ai_commands.commands[index];
      if (command) change(command);
    });
  }

  function add() {
    void store.save((s) => {
      s.ai_commands.commands.push(newCommand(s.ai_commands.commands));
    });
  }

  function remove(index: number) {
    void store.save((s) => {
      s.ai_commands.commands.splice(index, 1);
    });
  }
</script>

{#if store.settings}
  {@const ac = store.settings.ai_commands}
  {@const warnings = commandWarnings(ac.commands)}
  <Section title="Ask about what's on screen">
    <Row
      label="Ask about the selection"
      hint="Select text or files in any app, press this, then type your question. Click, then press the keys you want."
    >
      <span class="shortcut">
        <button
          type="button"
          class="field"
          class:recording
          onclick={() => (recording ? stopRecording(null) : startRecording())}
          onkeydown={onRecorderKey}
          onblur={() => recording && stopRecording(null)}
        >
          {recording ? "Press keys… (Esc to cancel)" : ac.selection_shortcut || "Off"}
        </button>
        {#if ac.selection_shortcut && !recording}
          <Button label="Turn the selection shortcut off" onclick={() => void store.save((s) => (s.ai_commands.selection_shortcut = ""))}
            >Off</Button
          >
        {/if}
      </span>
    </Row>
    <Row
      label="Put the clipboard back"
      hint="The selection is copied with Ctrl+C; afterwards the clipboard gets back what it held."
    >
      <Toggle
        on={ac.restore_clipboard}
        label="Put the clipboard back"
        onchange={(on) => void store.save((s) => (s.ai_commands.restore_clipboard = on))}
      />
    </Row>
    <Row
      label="Other ways in"
      hint="Ctrl+Shift+A on a file asks about it. “Ask About Screen Region” snips part of the screen. “Dictate” (or the mic in the search bar) starts Windows voice typing; Windows may ask you to turn it on first."
    />
  </Section>
  <Section title="Saved AI commands">
    <Row label="AI commands in search" hint={LIST_HINT}>
      <Toggle
        on={ac.enabled}
        label="AI commands in search"
        onchange={(on) => void store.save((s) => (s.ai_commands.enabled = on))}
      />
    </Row>
    {#if ac.enabled}
      {#each ac.commands as command, i (i)}
        <div class="command">
          <div class="head">
            <span class="name">
              <TextField
                value={command.name}
                placeholder="Name"
                label="Command name"
                onchange={(v) => edit(i, (c) => (c.name = v.trim()))}
              />
            </span>
            <span class="keyword">
              <TextField
                value={command.keyword}
                placeholder="keyword"
                label="Keyword"
                mono
                onchange={(v) => edit(i, (c) => (c.keyword = v.trim()))}
              />
            </span>
            <span class="icon">
              <TextField
                value={command.icon}
                placeholder="icon"
                label="Icon: a glyph name or an emoji"
                onchange={(v) => edit(i, (c) => (c.icon = v.trim()))}
              />
            </span>
            <Button label={`Remove ${command.name || "command"}`} onclick={() => remove(i)}>Remove</Button>
          </div>
          <div class="ai">
            <Choice
              options={AI_CHOICES}
              value={command.provider}
              label={`Who answers ${command.name || "this command"}`}
              onchange={(v) => edit(i, (c) => (c.provider = v as CommandAi))}
            />
          </div>
          <TextField
            value={command.prompt}
            placeholder={PROMPT_HINT}
            label="Prompt"
            multiline
            onchange={(v) => edit(i, (c) => (c.prompt = v))}
          />
          {#each warnings[i] ?? [] as warning (warning)}
            <small class="warning">{warning}</small>
          {/each}
        </div>
      {/each}
      <div class="add">
        <Button onclick={add}>Add command</Button>
        <small>{PLACEHOLDER_HINT}</small>
      </div>
    {/if}
  </Section>
{/if}

<style>
  .shortcut {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
  }
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
  .command {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--hairline);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .name,
  .keyword,
  .icon {
    display: flex;
    min-width: 0;
  }
  .name {
    flex: 3;
  }
  .keyword {
    flex: 2;
  }
  .icon {
    flex: 1;
  }
  .name :global(input),
  .keyword :global(input),
  .icon :global(input) {
    flex: 1;
  }
  .ai {
    display: flex;
  }
  .warning {
    font-size: var(--font-size-sub);
    color: var(--danger);
  }
  .add {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-1);
    padding: var(--space-2) 0;
  }
  .add small {
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
</style>
