<script lang="ts">
  // Clipboard history: on/off, how much to keep, images, programs to skip.
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import ChipList from "./ChipList.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Stepper from "./Stepper.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();

  function addApp(app: string) {
    const name = app.replace(/\.exe$/i, "");
    void store.save((s) => {
      const known = s.clipboard.ignore_apps.some((a) => a.toLowerCase() === name.toLowerCase());
      if (!known) s.clipboard.ignore_apps.push(name);
    });
  }

  function removeApp(app: string) {
    void store.save((s) => {
      s.clipboard.ignore_apps = s.clipboard.ignore_apps.filter((a) => a !== app);
    });
  }
</script>

{#if store.settings}
  {@const c = store.settings.clipboard}
  <Section title="Clipboard history">
    <Row label="Keep clipboard history" hint="Type clip in the launcher to search what you copied; Enter pastes it.">
      <Toggle
        on={c.enabled}
        label="Keep clipboard history"
        onchange={(on) => void store.save((s) => (s.clipboard.enabled = on))}
      />
    </Row>
    {#if c.enabled}
      <Row label="Entries kept" hint="Pinned entries (Ctrl+Shift+P) don't count and stay until you delete them.">
        <Stepper
          value={c.max_items}
          min={50}
          max={1000}
          step={50}
          onchange={(v) => void store.save((s) => (s.clipboard.max_items = v))}
        />
      </Row>
      <Row label="Keep images" hint="Screenshots and copied pictures, saved as PNG files.">
        <Toggle
          on={c.keep_images}
          label="Keep images"
          onchange={(on) => void store.save((s) => (s.clipboard.keep_images = on))}
        />
      </Row>
      <Row
        label="Never record copies from"
        hint="Program names. Copies an app marks as private (most password managers do) are never recorded either."
        stacked
      >
        <ChipList items={c.ignore_apps} placeholder="Add a program, e.g. Enpass" onadd={addApp} onremove={removeApp} />
      </Row>
    {/if}
  </Section>
{/if}
