<script lang="ts">
  import { commands } from "../../lib/bindings";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Banner from "./Banner.svelte";
  import Button from "./Button.svelte";
  import ChipList from "./ChipList.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Status from "./Status.svelte";
  import Stepper from "./Stepper.svelte";

  let { store }: { store: SettingsStore } = $props();

  function addTo(list: "index_folders" | "exclusions", item: string) {
    void store.save((s) => {
      if (!s.search[list].includes(item)) s.search[list].push(item);
    });
  }

  function removeFrom(list: "index_folders" | "exclusions", item: string) {
    void store.save((s) => {
      s.search[list] = s.search[list].filter((x) => x !== item);
    });
  }
</script>

{#if store.settings}
  {@const s = store.settings}
  {@const status = store.status}
  <Section title="Search">
    {#if status}
      <Status>
        <span>
          {#if status.everything}
            Everything is running: file search covers every drive.
          {:else if status.file_index.incomplete}
            Indexing your files… {status.file_index.entries.toLocaleString()} so far.
          {:else}
            Built-in index: {status.file_index.entries.toLocaleString()} files and folders.
          {/if}
        </span>
      </Status>
    {/if}
    <Row label="Folders to index" hint="Used when Everything isn't running. ~ is your user folder." stacked>
      <ChipList
        items={s.search.index_folders}
        placeholder="Add a folder, e.g. D:\Projects"
        onadd={(v) => addTo("index_folders", v)}
        onremove={(v) => removeFrom("index_folders", v)}
      />
    </Row>
    <Row label="Never show" hint="A folder name anywhere (node_modules) or part of a path." stacked>
      <ChipList
        items={s.search.exclusions}
        placeholder="Add a folder name or path"
        onadd={(v) => addTo("exclusions", v)}
        onremove={(v) => removeFrom("exclusions", v)}
      />
    </Row>
    {#if store.restartNeeded}
      <Banner>
        <span>Folder changes apply after a restart (it re-indexes).</span>
        <Button primary onclick={() => void commands.appRestart()}>Restart now</Button>
      </Banner>
    {/if}
    <Row label="Results shown">
      <Stepper
        value={s.search.max_results}
        min={4}
        max={20}
        onchange={(v) => void store.save((n) => (n.search.max_results = v))}
      />
    </Row>
  </Section>
{/if}
