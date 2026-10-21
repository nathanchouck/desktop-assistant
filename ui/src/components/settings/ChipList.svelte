<script lang="ts">
  interface Props {
    items: string[];
    placeholder: string;
    onadd: (value: string) => void;
    onremove: (item: string) => void;
  }
  let { items, placeholder, onadd, onremove }: Props = $props();
  let draft = $state("");

  function add() {
    const value = draft.trim();
    if (!value) return;
    onadd(value);
    draft = "";
  }
</script>

<div class="list">
  {#each items as item (item)}
    <span class="chip">
      {item}
      <button type="button" aria-label={`Remove ${item}`} onclick={() => onremove(item)}>×</button>
    </span>
  {/each}
  <input
    bind:value={draft}
    {placeholder}
    spellcheck="false"
    onkeydown={(e) => {
      if (e.key === "Enter") add();
    }}
  />
</div>

<style>
  .list {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 2px 2px 2px var(--space-2);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    font-family: var(--font-mono);
    font-size: var(--font-size-sub);
  }
  .chip button {
    padding: 0 var(--space-1);
    border: none;
    background: none;
    color: var(--text-muted);
    font: inherit;
  }
  input {
    flex: 1;
    min-width: 180px;
    padding: 2px var(--space-2);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--font-size-sub);
  }
  input:focus-visible,
  .chip button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
