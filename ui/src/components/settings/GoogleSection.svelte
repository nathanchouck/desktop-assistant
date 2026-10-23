<script lang="ts">
  // Google Calendar, Gmail and Drive through your own OAuth client: the
  // setup steps, the client ID and secret (the secret goes to Credential
  // Manager and never comes back), Connect / Disconnect, and options.
  import { commands, events, type GoogleStatus } from "../../lib/bindings";
  import {
    CLOUD_CONSOLE,
    MAX_DAYS,
    MIN_DAYS,
    SETUP_STEPS,
    clientIdWarning,
    connectBlocker,
    statusLine,
  } from "../../lib/google";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Button from "./Button.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Status from "./Status.svelte";
  import Stepper from "./Stepper.svelte";
  import TextField from "./TextField.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();

  let status = $state<GoogleStatus | null>(null);
  let secret = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);
  /** Disconnect was pressed once; again to confirm. */
  let confirming = $state(false);

  $effect(() => {
    void load();
    const unlisten = events.googleStatusChanged.listen((e) => {
      status = e.payload.status;
      busy = false;
    });
    return () => void unlisten.then((stop) => stop());
  });

  async function load() {
    const res = await commands.googleStatus();
    if (res.status === "ok") status = res.data;
  }

  async function run(call: () => ReturnType<typeof commands.googleStatus>) {
    busy = true;
    const res = await call();
    busy = false;
    if (res.status === "error") {
      error = res.error.message;
      return false;
    }
    error = null;
    status = res.data;
    return true;
  }

  async function saveSecret() {
    if (!secret.trim() || busy) return;
    if (await run(() => commands.googleSecretSet(secret))) secret = "";
  }

  async function disconnect() {
    if (!confirming) {
      confirming = true;
      return;
    }
    confirming = false;
    await run(() => commands.googleDisconnect());
  }
</script>

{#if store.settings}
  {@const g = store.settings.google}
  {@const blocker = connectBlocker(g, status)}
  {@const idWarning = clientIdWarning(g.client_id)}
  <Section title="Google">
    <Row
      label="Calendar, Gmail and Drive"
      hint="cal shows your next events (Enter joins the call), mail your unread mail, and Drive files show up in search. Read-only."
    >
      <Toggle on={g.enabled} label="Calendar, Gmail and Drive" onchange={(on) => void store.save((n) => (n.google.enabled = on))} />
    </Row>

    <Status>
      <span class:connected={status?.connected}>{statusLine(status)}</span>
      {#if status?.connected}
        <Button primary={confirming} disabled={busy} onclick={() => void disconnect()}>
          {confirming ? "Disconnect?" : "Disconnect"}
        </Button>
      {:else}
        <Button primary disabled={busy || blocker !== null || status?.connecting} onclick={() => void run(() => commands.googleConnect())}>
          {status?.connecting ? "Waiting…" : "Connect Google account"}
        </Button>
      {/if}
    </Status>
    {#if error ?? status?.problem}
      <p class="warning" role="alert">{error ?? status?.problem}</p>
    {/if}

    {#if !status?.connected}
      <Row label="Set up once" stacked>
        <ol class="steps">
          {#each SETUP_STEPS as step, i (i)}
            <li>{step}</li>
          {/each}
        </ol>
        <div class="console">
          <Button onclick={() => void commands.openExternal(CLOUD_CONSOLE)}>Open Google Cloud Console</Button>
        </div>
        <small class="note">
          In Testing mode Google ends the sign-in after 7 days, and Connect asks again. Publishing the app (Audience → Publish
          app) avoids that; Google then calls it unverified, which is fine for your own app.
        </small>
      </Row>
      <Row label="Client ID" hint={idWarning ?? "From the Desktop app client; it's not secret."} stacked>
        <TextField
          value={g.client_id}
          label="Client ID"
          placeholder="123-abc.apps.googleusercontent.com"
          mono
          onchange={(v) => void store.save((n) => (n.google.client_id = v.trim()))}
        />
      </Row>
      <Row label="Client secret" hint="Kept in Windows Credential Manager on this PC, never in the settings file." stacked>
        <div class="secret">
          <span class="state" class:saved={status?.secret_saved}>{status?.secret_saved ? "Saved" : "Not saved"}</span>
          <input
            type="password"
            bind:value={secret}
            placeholder={status?.secret_saved ? "Paste a new one to replace it" : "GOCSPX-…"}
            aria-label="Client secret"
            autocomplete="off"
            spellcheck="false"
            onkeydown={(e) => {
              if (e.key === "Enter") void saveSecret();
            }}
          />
          <Button primary disabled={!secret.trim() || busy} onclick={() => void saveSecret()}>Save</Button>
        </div>
      </Row>
      {#if blocker && status !== null}
        <small class="note">{blocker}</small>
      {/if}
    {/if}

    <Row label="Drive files in search" hint="What you type (3 or more letters) goes to Google Drive to find files by name.">
      <Toggle
        on={g.drive_in_search}
        label="Drive files in search"
        onchange={(on) => void store.save((n) => (n.google.drive_in_search = on))}
      />
    </Row>
    <Row label="Days in cal" hint="Today counts as one.">
      <Stepper
        value={g.calendar_days}
        min={MIN_DAYS}
        max={MAX_DAYS}
        onchange={(v) => void store.save((n) => (n.google.calendar_days = v))}
      />
    </Row>
  </Section>
{/if}

<style>
  .connected {
    color: var(--success);
  }
  .warning {
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-sub);
    color: var(--danger);
  }
  .steps {
    margin: 0;
    padding-left: var(--space-5);
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
  .steps li + li {
    margin-top: var(--space-1);
  }
  .console {
    display: flex;
  }
  .note {
    display: block;
    font-size: var(--font-size-sub);
    color: var(--text-faint);
  }
  .secret {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .state {
    flex: none;
    min-width: 6em;
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
</style>
