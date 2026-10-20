<script lang="ts">
  import type { ResultItem } from "../lib/bindings";
  import { detailFor } from "../lib/display";
  import ItemIcon from "./ItemIcon.svelte";

  interface Props {
    items: ResultItem[];
    selected: number;
    onselect: (index: number) => void;
    onactivate: (index: number) => void;
  }
  let { items, selected, onselect, onactivate }: Props = $props();
  let list: HTMLUListElement;

  const kindLabel: Record<ResultItem["kind"], string> = {
    app: "Application",
    file: "File",
    folder: "Folder",
    command: "Command",
    ask_ai: "Tab",
  };

  // Keep the highlighted row visible when there are more rows than fit.
  $effect(() => {
    list.querySelector(`#result-${selected}`)?.scrollIntoView({ block: "nearest" });
  });
</script>

<ul id="results" role="listbox" aria-label="Results" bind:this={list}>
  {#each items as item, i (item.id)}
    {@const detail = detailFor(item)}
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
      <span class="text">
        <span class="title">{item.title}</span>
        {#if detail}
          <span class="detail">{detail}</span>
        {/if}
      </span>
      <span class="hint">
        {#if i === selected || i >= 9}
          {kindLabel[item.kind]}
        {:else}
          Ctrl {i + 1}
        {/if}
      </span>
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
  .text {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
  }
  .title {
    flex: none;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--font-size-title);
  }
  .detail {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--font-size-sub);
    color: var(--text-faint);
  }
  .hint {
    flex: none;
    font-size: var(--font-size-sub);
    color: var(--text-faint);
  }
</style>
