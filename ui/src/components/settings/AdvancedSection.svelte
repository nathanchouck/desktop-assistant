<script lang="ts">
  import { commands } from "../../lib/bindings";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Button from "./Button.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";

  let { store }: { store: SettingsStore } = $props();
</script>

{#if store.settings}
  <Section title="Advanced">
    <Row label="Settings file" hint="Everything here, as text, for anything this menu doesn't cover.">
      <Button onclick={() => void commands.itemActivate("cmd:settings-file", "run")}>Open</Button>
    </Row>
    <Row label="Log files" hint="What the launcher did, for when something goes wrong.">
      <Button onclick={() => void commands.openLogFolder()}>Open</Button>
    </Row>
  </Section>
{/if}
