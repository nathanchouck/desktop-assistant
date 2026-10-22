<script lang="ts">
  // Color picker: on/off, how picked colors are copied, what happens after a pick.
  import type { ColorFormat } from "../../lib/bindings";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Choice from "./Choice.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();

  const formats: [ColorFormat, string][] = [
    ["hex", "#ff8800"],
    ["rgb", "rgb()"],
    ["hsl", "hsl()"],
  ];
</script>

{#if store.settings}
  {@const c = store.settings.color_picker}
  <Section title="Color picker">
    <Row
      label="Color picker"
      hint="Pick Color, then click anywhere on screen (Enter picks under the cursor, Esc cancels). Type colors to see the last 20."
    >
      <Toggle
        on={c.enabled}
        label="Color picker"
        onchange={(on) => void store.save((s) => (s.color_picker.enabled = on))}
      />
    </Row>
    {#if c.enabled}
      <Row label="Copy as" hint="Typing a color (#ff8800, rgb(…), hsl(…)) shows the other formats too.">
        <Choice
          options={formats}
          value={c.format}
          label="Copy as"
          onchange={(v) => void store.save((s) => (s.color_picker.format = v as ColorFormat))}
        />
      </Row>
      <Row label="Show the color after picking" hint="Opens the launcher on your recent colors. Off: the color is only copied.">
        <Toggle
          on={c.show_after_pick}
          label="Show the color after picking"
          onchange={(on) => void store.save((s) => (s.color_picker.show_after_pick = on))}
        />
      </Row>
    {/if}
  </Section>
{/if}
