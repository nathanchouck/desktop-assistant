<script lang="ts">
  // System commands (lock, sleep, restart, ...) and whether the risky ones ask first.
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();
</script>

{#if store.settings}
  {@const s = store.settings.system}
  <Section title="System">
    <Row
      label="System commands"
      hint="Lock, sleep, restart, shut down, empty the Recycle Bin, dark mode, mute, and Windows Settings pages (Wi-Fi, Bluetooth, Display…)."
    >
      <Toggle
        on={s.enabled}
        label="System commands"
        onchange={(on) => void store.save((x) => (x.system.enabled = on))}
      />
    </Row>
    {#if s.enabled}
      <Row
        label="Ask before risky commands"
        hint="Restart, shut down, sign out, hibernate and emptying the Recycle Bin wait for Enter a second time."
      >
        <Toggle
          on={s.confirm}
          label="Ask before risky commands"
          onchange={(on) => void store.save((x) => (x.system.confirm = on))}
        />
      </Row>
    {/if}
    <Row
      label="Also in search"
      hint="Window layouts for the window you came from (left half, center, next monitor…), media keys (play / pause, next track), and kill followed by a program's name to end it."
    />
  </Section>
{/if}
