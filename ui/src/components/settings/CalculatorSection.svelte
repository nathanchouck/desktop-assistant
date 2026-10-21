<script lang="ts">
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();
</script>

{#if store.settings}
  {@const s = store.settings}
  <Section title="Calculator">
    <Row label="Answers in search" hint="2^10, 15% of 80, 5 mi to km, 72 f to c, 255 to hex. Start with = to always get an answer.">
      <Toggle
        on={s.calculator.enabled}
        label="Answers in search"
        onchange={(on) => void store.save((n) => (n.calculator.enabled = on))}
      />
    </Row>
    <Row label="Currencies" hint="100 usd to eur, €50 in $. European Central Bank rates, downloaded once a day.">
      <Toggle
        on={s.calculator.currency}
        label="Currencies"
        onchange={(on) => void store.save((n) => (n.calculator.currency = on))}
      />
    </Row>
  </Section>
{/if}
