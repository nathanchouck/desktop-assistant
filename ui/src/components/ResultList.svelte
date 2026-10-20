<script lang="ts">
  import type { ResultItem } from "../lib/bindings";
  import ItemIcon from "./ItemIcon.svelte";

  interface Props {
    items: ResultItem[];
    selected: number;
    onselect: (index: number) => void;
    onactivate: (index: number) => void;
  }
  let { items, selected, onselect, onactivate }: Props = $props();
  let list: HTMLUListElement;

  // Keep the highlighted row visible when there are more rows than fit.
  $effect(() => {
    list.querySelector(`#result-${selected}`)?.scrollIntoView({ block: "nearest" });
  });
</script>

<ul id="results" role="listbox" aria-label="Results" bind:this={list}>
  {#each items as item, i (item.id)}
    <!-- Keyboard is handled by the search input (combobox pattern); rows only take the mouse. -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <li
      id={`result-${i}`}
      role="option"
      aria-selected={i === selected}
      class:selected={i === selected}
      onmousemove={() => onselect(i)}
      onclick={() => onactivate(i)}
    >
      <ItemIcon kind={item.kind} icon={item.icon} />
      <span class="title">{item.title}</span>
      {#if i === selected || i >= 9}
        <span class="subtitle">{item.subtitle}</span>
      {:else}
        <span class="subtitle">Ctrl {i + 1}</span>
      {/if}
    </li>
  {/each}
</ul>

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: var(--space-1) var(--space-2);
    height: calc(var(--row-height) * var(--results-rows) + var(--space-2));
    overflow-y: auto;
    scrollbar-width: none;
  }
  li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    height: var(--row-height);
    padding: 0 var(--space-3);
    border: 1px solid transparent;
    border-radius: var(--radius-row);
    cursor: default;
    transition: background var(--duration-fast) var(--ease);
  }
  li.selected {
    background: var(--selection-sheen), var(--selection);
    border-color: var(--selection-border);
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--font-size-title);
  }
  .subtitle {
    flex: none;
    font-size: var(--font-size-sub);
    color: var(--text-faint);
  }
</style>
