<script lang="ts">
  import { events } from "../../lib/bindings";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import { UpdatesStore } from "../../lib/stores/updates.svelte";
  import { canCheck, readyVersion, statusLine } from "../../lib/updates";
  import Button from "./Button.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();

  const updates = new UpdatesStore();
  /** For "checked 5 minutes ago"; ticks while the menu is open. */
  let now = $state(Date.now());

  $effect(() => {
    void updates.load();
    const unlisten = events.updateStatusChanged.listen((e) => updates.apply(e.payload.status));
    const timer = setInterval(() => (now = Date.now()), 30_000);
    return () => {
      clearInterval(timer);
      void unlisten.then((stop) => stop());
    };
  });
</script>

{#if store.settings && updates.status}
  {@const s = store.settings}
  {@const status = updates.status}
  {@const ready = readyVersion(status.phase)}
  {@const setUp = status.phase.state !== "not_set_up"}
  <Section title="Updates">
    <Row label={`Version ${status.current_version}`} hint={statusLine(status, now)}>
      {#if setUp}
        <span class="buttons">
          <Button disabled={!canCheck(status.phase)} onclick={() => void updates.check()}>Check now</Button>
          {#if ready}
            <Button primary onclick={() => void updates.install()}>Install & restart</Button>
          {/if}
        </span>
      {/if}
    </Row>
    {#if updates.error}
      <p class="error" role="alert">{updates.error}</p>
    {/if}
    {#if setUp}
      <Row
        label="Check automatically"
        hint="A minute after start and every 6 hours, from GitHub. New versions download in the background; installing waits for you."
      >
        <Toggle
          on={s.updates.check_automatically}
          label="Check automatically"
          onchange={(on) => void store.save((n) => (n.updates.check_automatically = on))}
        />
      </Row>
    {:else}
      <p class="note">Only copies installed from a signed GitHub release update themselves.</p>
    {/if}
  </Section>
{/if}

<style>
  .buttons {
    display: flex;
    flex: none;
    gap: var(--space-2);
  }
  .error,
  .note {
    margin: var(--space-2) 0 0;
    font-size: var(--font-size-sub);
  }
  .error {
    color: var(--danger);
  }
  .note {
    color: var(--text-muted);
  }
</style>
