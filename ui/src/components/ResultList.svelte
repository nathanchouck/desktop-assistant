<script lang="ts">
  import type { ResultItem } from "../lib/bindings";

  interface Props {
    items: ResultItem[];
    selected: number;
    onselect: (index: number) => void;
    onactivate: (index: number) => void;
  }
  let { items, selected, onselect, onactivate }: Props = $props();

  const badge: Record<ResultItem["kind"], string> = {
    app: "App",
    file: "File",
    folder: "Folder",
    command: "Cmd",
    ask_ai: "AI",
  };
</script>

<ul id="results" role="listbox" aria-label="Results">
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
      <span class="badge" class:ai={item.kind === "ask_ai"}>{badge[item.kind]}</span>
      <span class="title">{item.title}</span>
      <span class="subtitle">{item.subtitle}</span>
    </li>
  {/each}
</ul>

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 0 var(--space-2) var(--space-2);
  }
  li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: 1px solid transparent;
    border-radius: var(--radius-row);
    cursor: default;
    transition: background var(--duration-fast) var(--ease);
  }
  li.selected {
    background: var(--selection-sheen), var(--selection);
    border-color: var(--selection-border);
  }
  .badge {
    flex: none;
    width: 44px;
    text-align: center;
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
  .badge.ai {
    color: var(--accent);
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
