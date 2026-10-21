<script lang="ts">
  import type { SkinTone } from "../../lib/bindings";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Choice from "./Choice.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();

  const TONES: [SkinTone, string][] = [
    ["none", "👋"],
    ["light", "👋🏻"],
    ["medium_light", "👋🏼"],
    ["medium", "👋🏽"],
    ["medium_dark", "👋🏾"],
    ["dark", "👋🏿"],
  ];
</script>

{#if store.settings}
  {@const s = store.settings}
  <Section title="Emoji">
    <Row label="Emoji and symbols" hint="Type : and a name (:fire, :arrow, :degree). Enter pastes it, Ctrl+Shift+C copies.">
      <Toggle
        on={s.emoji.enabled}
        label="Emoji and symbols"
        onchange={(on) => void store.save((n) => (n.emoji.enabled = on))}
      />
    </Row>
    <Row label="Skin tone" hint="For people and hands.">
      <Choice
        options={TONES}
        value={s.emoji.skin_tone}
        label="Skin tone"
        onchange={(tone) => void store.save((n) => (n.emoji.skin_tone = tone as SkinTone))}
      />
    </Row>
  </Section>
{/if}
