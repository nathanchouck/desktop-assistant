<script lang="ts">
  import type { Density, GlassStrength } from "../../lib/bindings";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import { normalizeTint, parseHex, PRESET_NAMES, PRESETS, toHex } from "../../lib/theme";
  import Choice from "./Choice.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import TextField from "./TextField.svelte";

  let { store }: { store: SettingsStore } = $props();
  const tint = $derived(normalizeTint(store.settings?.appearance.tint ?? ""));
  /** The custom color in use, if the tint is one. */
  const custom = $derived(tint.startsWith("#") ? tint : "");
  let customError = $state<string | null>(null);
  /** Glass strength only shows while Windows' blur is behind the window
   * (set once at startup by main.ts). */
  const glassOn = document.documentElement.dataset.glass === "on";

  const GLASS: [GlassStrength, string][] = [
    ["subtle", "Subtle"],
    ["normal", "Normal"],
    ["strong", "Strong"],
  ];
  const DENSITY: [Density, string][] = [
    ["comfortable", "Comfortable"],
    ["compact", "Compact"],
  ];

  function setTint(value: string) {
    customError = null;
    void store.save((s) => (s.appearance.tint = value));
  }

  function setCustom(text: string) {
    if (!text.trim()) {
      customError = null;
      return;
    }
    const rgb = parseHex(text);
    if (rgb) setTint(toHex(rgb));
    else customError = "That isn't a color. Type one like #ff8800.";
  }
</script>

{#if store.settings}
  <Section title="Appearance">
    <Row label="Accent color" hint="Buttons, switches and links.">
      <div class="swatches" role="radiogroup" aria-label="Accent color">
        {#each PRESETS as name (name)}
          <button
            type="button"
            role="radio"
            class="swatch"
            class:on={tint === name}
            aria-checked={tint === name}
            aria-label={PRESET_NAMES[name]}
            title={PRESET_NAMES[name]}
            style:--swatch={`var(--tint-${name})`}
            onclick={() => setTint(name)}
          ></button>
        {/each}
      </div>
    </Row>
    <Row
      label="Custom color"
      hint={customError ?? "Any color as #rrggbb. It's made darker or lighter where text has to stay readable."}
    >
      <div class="custom">
        {#if custom}
          <span class="swatch on" style:--swatch="var(--accent)" aria-hidden="true"></span>
        {/if}
        <TextField value={custom} placeholder="#rrggbb" label="Custom accent color" mono onchange={setCustom} />
      </div>
    </Row>
    <Row
      label="Glass"
      hint={glassOn
        ? "How much of the desktop shows through the window."
        : "Windows' blur isn't available here, so the window is solid."}
    >
      <Choice
        options={GLASS}
        value={store.settings.appearance.glass}
        label="Glass"
        onchange={(v) => void store.save((s) => (s.appearance.glass = v as GlassStrength))}
      />
    </Row>
    <Row label="Density" hint="Compact rows are shorter, so the window takes less room.">
      <Choice
        options={DENSITY}
        value={store.settings.appearance.density}
        label="Density"
        onchange={(v) => void store.save((s) => (s.appearance.density = v as Density))}
      />
    </Row>
  </Section>
{/if}

<style>
  .swatches,
  .custom {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
  }
  .swatch {
    flex: none;
    width: var(--swatch-size);
    height: var(--swatch-size);
    padding: 0;
    border: none;
    border-radius: 50%;
    background: var(--swatch);
    box-shadow: inset 0 0 0 1px var(--swatch-ring);
  }
  .swatch.on {
    outline: 2px solid var(--swatch);
    outline-offset: 2px;
  }
  .swatch:focus-visible {
    outline: 2px solid var(--text);
    outline-offset: 2px;
  }
  .custom :global(input) {
    width: 10ch;
  }
</style>
