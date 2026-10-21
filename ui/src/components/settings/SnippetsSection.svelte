<script lang="ts">
  // Snippets: the list (name, keyword, text) and expansion as you type.
  import type { Snippet as SavedSnippet } from "../../lib/bindings";
  import { PLACEHOLDERS, snippetWarnings } from "../../lib/snippets";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Button from "./Button.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import TextField from "./TextField.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();

  const TEXT_HINT = `The text. Placeholders: ${PLACEHOLDERS.join(" ")}`;

  function edit(index: number, change: (snippet: SavedSnippet) => void) {
    void store.save((s) => {
      const snippet = s.snippets.items[index];
      if (snippet) change(snippet);
    });
  }

  function add() {
    void store.save((s) => {
      s.snippets.items.push({ name: "New snippet", keyword: "", text: "" });
    });
  }

  function remove(index: number) {
    void store.save((s) => {
      s.snippets.items.splice(index, 1);
    });
  }
</script>

{#if store.settings}
  {@const sn = store.settings.snippets}
  {@const warnings = snippetWarnings(sn.items, sn.trigger, sn.enabled && sn.expand_as_you_type)}
  <Section title="Snippets">
    <Row label="Snippets in search" hint="Find one by name or keyword and press Enter to paste it. Type snip to list them all.">
      <Toggle
        on={sn.enabled}
        label="Snippets in search"
        onchange={(on) => void store.save((s) => (s.snippets.enabled = on))}
      />
    </Row>
    {#if sn.enabled}
      <Row
        label="Expand as you type"
        hint="Typing a keyword in any app replaces it with its snippet (pasted through the clipboard, which is put back after)."
      >
        <Toggle
          on={sn.expand_as_you_type}
          label="Expand as you type"
          onchange={(on) => void store.save((s) => (s.snippets.expand_as_you_type = on))}
        />
      </Row>
      {#if sn.expand_as_you_type}
        <Row label="Keywords start with" hint="Only these keywords expand, so ordinary words never do. Empty: all of them.">
          <TextField
            value={sn.trigger}
            label="Keywords start with"
            mono
            onchange={(v) => void store.save((s) => (s.snippets.trigger = v.trim()))}
          />
        </Row>
      {/if}
      {#each sn.items as item, i (i)}
        <div class="snippet">
          <div class="head">
            <span class="name">
              <TextField
                value={item.name}
                placeholder="Name"
                label="Snippet name"
                onchange={(v) => edit(i, (s) => (s.name = v.trim()))}
              />
            </span>
            <span class="keyword">
              <TextField
                value={item.keyword}
                placeholder={`${sn.trigger || ";"}keyword`}
                label="Keyword"
                mono
                onchange={(v) => edit(i, (s) => (s.keyword = v.trim()))}
              />
            </span>
            <Button label={`Remove ${item.name || "snippet"}`} onclick={() => remove(i)}>Remove</Button>
          </div>
          <TextField
            value={item.text}
            placeholder={TEXT_HINT}
            label="Snippet text"
            multiline
            onchange={(v) => edit(i, (s) => (s.text = v))}
          />
          {#each warnings[i] ?? [] as warning (warning)}
            <small class="warning">{warning}</small>
          {/each}
        </div>
      {/each}
      <div class="add">
        <Button onclick={add}>Add snippet</Button>
      </div>
    {/if}
  </Section>
{/if}

<style>
  .snippet {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--hairline);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .name,
  .keyword {
    display: flex;
    min-width: 0;
  }
  .name {
    flex: 2;
  }
  .keyword {
    flex: 1;
  }
  .name :global(input),
  .keyword :global(input) {
    flex: 1;
  }
  .warning {
    font-size: var(--font-size-sub);
    color: var(--danger);
  }
  .add {
    padding: var(--space-2) 0;
  }
</style>
