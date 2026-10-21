<script lang="ts">
  import { commands, type ClaudeStatus } from "../lib/bindings";
  import { shortcutFromEvent } from "../lib/keys";
  import type { SettingsStore } from "../lib/stores/settings.svelte";

  interface Props {
    store: SettingsStore;
    onclose: () => void;
  }
  let { store, onclose }: Props = $props();
  let pane: HTMLDivElement;
  let recording = $state(false);
  let newFolder = $state("");
  let newExclusion = $state("");

  const MODELS: [string, string][] = [
    ["sonnet", "Sonnet 5.5"],
    ["haiku", "Haiku 5.5"],
  ];
  const CLAUDE_STATUS: Record<ClaudeStatus, string> = {
    ready: "Claude Code is installed and signed in.",
    not_installed: "Claude Code isn't installed.",
    signed_out: "Claude Code isn't signed in.",
    unknown: "Checking Claude Code…",
  };

  /** Focus the menu so the keyboard works in it. */
  export function focus(): void {
    pane.focus();
  }

  // Keep the status lines fresh while the menu is open.
  $effect(() => {
    const timer = setInterval(() => void store.refreshStatus(), 2000);
    return () => clearInterval(timer);
  });

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

  function onkeydown(e: KeyboardEvent) {
    if (recording) {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") void stopRecording(null);
      else {
        const shortcut = shortcutFromEvent(e);
        if (shortcut) void stopRecording(shortcut);
      }
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      onclose();
    }
  }

  function addTo(list: "index_folders" | "exclusions", value: string) {
    const item = value.trim();
    if (!item) return;
    void store.save((s) => {
      if (!s.search[list].includes(item)) s.search[list].push(item);
    });
  }

  function removeFrom(list: "index_folders" | "exclusions", item: string) {
    void store.save((s) => {
      s.search[list] = s.search[list].filter((x) => x !== item);
    });
  }

  function seen(secs: number | null): string {
    if (secs === null) return "hasn't seen the key yet";
    if (secs < 60) return `saw it ${secs} s ago`;
    if (secs < 3600) return `saw it ${Math.round(secs / 60)} min ago`;
    return `saw it ${Math.round(secs / 3600)} h ago`;
  }
</script>

{#snippet toggle(on: boolean, label: string, onchange: (on: boolean) => void)}
  <button type="button" role="switch" aria-checked={on} aria-label={label} class="switch" class:on onclick={() => onchange(!on)}>
    <span class="knob"></span>
  </button>
{/snippet}

{#snippet choice(options: [string, string][], value: string, label: string, onchange: (v: string) => void)}
  <div class="segmented" role="radiogroup" aria-label={label}>
    {#each options as [key, text] (key)}
      <button type="button" role="radio" aria-checked={value === key} class:on={value === key} onclick={() => onchange(key)}>
        {text}
      </button>
    {/each}
  </div>
{/snippet}

{#snippet list(kind: "index_folders" | "exclusions", items: string[], placeholder: string)}
  <div class="list">
    {#each items as item (item)}
      <span class="chip">
        {item}
        <button type="button" aria-label={`Remove ${item}`} onclick={() => removeFrom(kind, item)}>×</button>
      </span>
    {/each}
    {#if kind === "index_folders"}
      <input
        bind:value={newFolder}
        {placeholder}
        spellcheck="false"
        onkeydown={(e) => {
          if (e.key === "Enter") {
            addTo(kind, newFolder);
            newFolder = "";
          }
        }}
      />
    {:else}
      <input
        bind:value={newExclusion}
        {placeholder}
        spellcheck="false"
        onkeydown={(e) => {
          if (e.key === "Enter") {
            addTo(kind, newExclusion);
            newExclusion = "";
          }
        }}
      />
    {/if}
  </div>
{/snippet}

<!-- Esc and shortcut recording are handled for the whole menu. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="pane" bind:this={pane} tabindex="-1" {onkeydown} role="region" aria-label="Settings">
  {#if store.error}
    <p class="error" role="alert">{store.error}</p>
  {/if}
  {#if store.settings}
    {@const s = store.settings}
    {@const status = store.status}

    <h3>Opening the launcher</h3>
    <div class="row">
      <div class="label">
        <span>Shortcut</span>
        <small>Click, then press the keys you want.</small>
      </div>
      <button type="button" class="field" class:recording onclick={() => (recording ? stopRecording(null) : startRecording())}>
        {recording ? "Press keys… (Esc to cancel)" : s.hotkeys.toggle}
      </button>
    </div>
    <div class="row">
      <div class="label">
        <span>Tap the Windows key</span>
        <small>
          Opens the launcher instead of Start.
          {#if status?.win_key.enabled}
            {status.win_key.running ? `Working: ${seen(status.win_key.last_seen_secs)}.` : "Not running."}
          {/if}
        </small>
      </div>
      {@render toggle(s.hotkeys.win_key_override, "Tap the Windows key", (on) =>
        void store.save((n) => (n.hotkeys.win_key_override = on)),
      )}
    </div>
    <div class="row">
      <div class="label"><span>Start with Windows</span></div>
      {@render toggle(s.general.launch_at_login, "Start with Windows", (on) =>
        void store.save((n) => (n.general.launch_at_login = on)),
      )}
    </div>

    <h3>Claude</h3>
    {#if status}
      <div class="status">
        <span>{CLAUDE_STATUS[status.claude]}</span>
        {#if status.claude === "not_installed"}
          <button type="button" onclick={() => void commands.itemActivate("cmd:claude-install", "run")}>Install</button>
        {:else if status.claude === "signed_out"}
          <button type="button" onclick={() => void commands.itemActivate("cmd:claude-sign-in", "run")}>Sign in</button>
        {/if}
      </div>
    {/if}
    <div class="row">
      <div class="label">
        <span>Model</span>
        <small>Sonnet is smarter; Haiku is faster and uses less of your plan.</small>
      </div>
      {@render choice(
        MODELS.some(([key]) => key === s.ai.claude.model) ? MODELS : [...MODELS, [s.ai.claude.model, s.ai.claude.model]],
        s.ai.claude.model,
        "Model",
        (model) => void store.save((n) => (n.ai.claude.model = model)),
      )}
    </div>
    <div class="row">
      <div class="label">
        <span>Approvals</span>
        <small>
          {s.ai.claude.approvals === "ask"
            ? "You approve anything that changes files or runs commands."
            : "Claude Code decides routine actions; it asks when unsure. Screenshots always ask."}
        </small>
      </div>
      {@render choice(
        [
          ["ask", "Ask me"],
          ["auto", "Auto"],
        ],
        s.ai.claude.approvals,
        "Approvals",
        (v) => void store.save((n) => (n.ai.claude.approvals = v as "ask" | "auto")),
      )}
    </div>
    <div class="row">
      <div class="label">
        <span>Screen context</span>
        <small>Tell Claude which window you came from, and Explorer's folder and selection.</small>
      </div>
      {@render toggle(s.ai.claude.desktop_context, "Screen context", (on) =>
        void store.save((n) => (n.ai.claude.desktop_context = on)),
      )}
    </div>
    <div class="row">
      <div class="label">
        <span>Start Claude early</span>
        <small>While you type a question, so the answer starts about a second sooner.</small>
      </div>
      {@render toggle(s.ai.claude.warm_start, "Start Claude early", (on) =>
        void store.save((n) => (n.ai.claude.warm_start = on)),
      )}
    </div>
    <div class="row">
      <div class="label">
        <span>Billing</span>
        <small>
          {s.ai.claude.billing === "subscription"
            ? "Uses your Claude plan through Claude Code's sign-in."
            : "Uses the ANTHROPIC_API_KEY environment variable."}
        </small>
      </div>
      {@render choice(
        [
          ["subscription", "My plan"],
          ["api_key", "API key"],
        ],
        s.ai.claude.billing,
        "Billing",
        (v) => void store.save((n) => (n.ai.claude.billing = v as "subscription" | "api_key")),
      )}
    </div>

    <h3>Search</h3>
    {#if status}
      <div class="status">
        <span>
          {#if status.everything}
            Everything is running: file search covers every drive.
          {:else if status.file_index.incomplete}
            Indexing your files… {status.file_index.entries.toLocaleString()} so far.
          {:else}
            Built-in index: {status.file_index.entries.toLocaleString()} files and folders.
          {/if}
        </span>
      </div>
    {/if}
    <div class="row stacked">
      <div class="label">
        <span>Folders to index</span>
        <small>Used when Everything isn't running. ~ is your user folder.</small>
      </div>
      {@render list("index_folders", s.search.index_folders, "Add a folder, e.g. D:\\Projects")}
    </div>
    <div class="row stacked">
      <div class="label">
        <span>Never show</span>
        <small>A folder name anywhere (node_modules) or part of a path.</small>
      </div>
      {@render list("exclusions", s.search.exclusions, "Add a folder name or path")}
    </div>
    {#if store.restartNeeded}
      <div class="banner">
        <span>Folder changes apply after a restart (it re-indexes).</span>
        <button type="button" class="primary" onclick={() => void commands.appRestart()}>Restart now</button>
      </div>
    {/if}
    <div class="row">
      <div class="label"><span>Results shown</span></div>
      <div class="stepper">
        <button
          type="button"
          aria-label="Fewer"
          disabled={s.search.max_results <= 4}
          onclick={() => void store.save((n) => (n.search.max_results -= 1))}>−</button
        >
        <span>{s.search.max_results}</span>
        <button
          type="button"
          aria-label="More"
          disabled={s.search.max_results >= 20}
          onclick={() => void store.save((n) => (n.search.max_results += 1))}>+</button
        >
      </div>
    </div>

    <div class="row">
      <div class="label">
        <span>Settings file</span>
        <small>Everything here, as text, for anything this menu doesn't cover.</small>
      </div>
      <button type="button" onclick={() => void commands.itemActivate("cmd:settings-file", "run")}>Open</button>
    </div>
  {/if}
</div>

<style>
  .pane {
    height: calc(var(--row-height) * var(--results-rows) + var(--space-2));
    padding: 0 var(--space-5) var(--space-4);
    overflow-y: auto;
    scrollbar-width: thin;
    outline: none;
    font-size: var(--font-size-title);
  }
  h3 {
    margin: var(--space-4) 0 var(--space-1);
    font-size: var(--font-size-sub);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--hairline);
  }
  .row.stacked {
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-2);
  }
  .label {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  small {
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
  .status {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-1) 0 var(--space-2);
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
  button {
    flex: none;
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    color: var(--text);
    font: inherit;
    font-size: var(--font-size-sub);
  }
  button:disabled {
    opacity: 0.4;
  }
  button:focus-visible,
  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  button.primary {
    border-color: transparent;
    background: var(--accent);
    color: var(--on-accent);
  }
  .field {
    min-width: 120px;
    font-family: var(--font-mono);
  }
  .field.recording {
    border-color: var(--accent);
    color: var(--accent);
  }

  .switch {
    position: relative;
    width: 36px;
    height: 20px;
    padding: 0;
    border-radius: 10px;
    background: var(--switch-off);
    transition: background var(--duration-fast) var(--ease);
  }
  .switch.on {
    border-color: transparent;
    background: var(--accent);
  }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--switch-knob);
    transition: transform var(--duration-fast) var(--ease);
  }
  .switch.on .knob {
    transform: translateX(16px);
  }

  .segmented {
    display: flex;
    flex: none;
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    overflow: hidden;
  }
  .segmented button {
    border: none;
    border-radius: 0;
    background: transparent;
    color: var(--text-muted);
  }
  .segmented button.on {
    background: var(--accent);
    color: var(--on-accent);
  }

  .list {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 2px 2px 2px var(--space-2);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    font-family: var(--font-mono);
    font-size: var(--font-size-sub);
  }
  .chip button {
    padding: 0 var(--space-1);
    border: none;
    background: none;
    color: var(--text-muted);
  }
  .list input {
    flex: 1;
    min-width: 180px;
    padding: 2px var(--space-2);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--font-size-sub);
  }

  .stepper {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .stepper span {
    min-width: 2ch;
    text-align: center;
  }

  .error {
    margin: var(--space-2) 0 0;
    font-size: var(--font-size-sub);
    color: var(--danger);
  }
  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    margin-top: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--approval-border);
    border-radius: var(--radius-row);
    background: var(--approval);
    font-size: var(--font-size-sub);
  }
</style>
