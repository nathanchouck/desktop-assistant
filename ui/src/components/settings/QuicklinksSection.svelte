<script lang="ts">
  import type { Quicklink } from "../../lib/bindings";
  import { cleanLink, linkProblem } from "../../lib/quicklinks";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Button from "./Button.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import TextField from "./TextField.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();

  const EMPTY: Quicklink = { keyword: "", name: "", url: "" };
  let draft = $state<Quicklink>({ ...EMPTY });
  /** Why the last edit or add wasn't saved. */
  let problem = $state<string | null>(null);

  function links(): Quicklink[] {
    return store.settings?.quicklinks.links ?? [];
  }

  function edit(index: number, field: keyof Quicklink, value: string) {
    const old = links()[index];
    if (!old) return;
    const link = cleanLink({ ...old, [field]: value });
    problem = linkProblem(link, links(), index);
    if (problem) return;
    void store.save((s) => {
      s.quicklinks.links[index] = link;
    });
  }

  function remove(index: number) {
    problem = null;
    void store.save((s) => {
      s.quicklinks.links.splice(index, 1);
    });
  }

  async function add() {
    const link = cleanLink(draft);
    problem = linkProblem(link, links());
    if (problem) return;
    const saved = await store.save((s) => {
      s.quicklinks.links.push(link);
    });
    if (saved) draft = { ...EMPTY };
  }
</script>

{#if store.settings}
  {@const s = store.settings}
  <Section title="Quicklinks">
    <Row label="Quicklinks" hint="Type a keyword and what to search for: yt lofi, gh tauri, maps coffee.">
      <Toggle
        on={s.quicklinks.enabled}
        label="Quicklinks"
        onchange={(on) => void store.save((n) => (n.quicklinks.enabled = on))}
      />
    </Row>
    <Row label="Links" hint="{'{query}'} in the address is replaced by what you type. Without it, the link just opens." stacked>
      <div class="grid">
        <span class="head">Keyword</span>
        <span class="head">Name</span>
        <span class="head">Address</span>
        <span></span>
        {#each s.quicklinks.links as link, i (`${i}:${link.keyword}`)}
          <TextField value={link.keyword} label={`Keyword for ${link.name}`} mono onchange={(v) => edit(i, "keyword", v)} />
          <TextField value={link.name} label={`Name for ${link.keyword}`} onchange={(v) => edit(i, "name", v)} />
          <TextField value={link.url} label={`Address for ${link.name}`} mono onchange={(v) => edit(i, "url", v)} />
          <Button label={`Remove ${link.name}`} onclick={() => remove(i)}>×</Button>
        {/each}
        <TextField value={draft.keyword} placeholder="yt" label="New keyword" mono onchange={(v) => (draft.keyword = v)} />
        <TextField value={draft.name} placeholder="YouTube" label="New name" onchange={(v) => (draft.name = v)} />
        <TextField
          value={draft.url}
          placeholder={"https://www.youtube.com/results?search_query={query}"}
          label="New address"
          mono
          onchange={(v) => (draft.url = v)}
        />
        <Button primary onclick={() => void add()}>Add</Button>
      </div>
      {#if problem}
        <p class="problem" role="alert">{problem}</p>
      {/if}
    </Row>
  </Section>
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 0.7fr) minmax(0, 1.1fr) minmax(0, 2.6fr) auto;
    gap: var(--space-1) var(--space-2);
    align-items: center;
  }
  .grid :global(input) {
    width: 100%;
  }
  .head {
    font-size: var(--font-size-sub);
    color: var(--text-faint);
  }
  .problem {
    margin: 0;
    font-size: var(--font-size-sub);
    color: var(--danger);
  }
</style>
