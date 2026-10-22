<script lang="ts">
  import type { ActionMenuStore } from "../lib/stores/actionMenu.svelte";

  interface Props {
    menu: ActionMenuStore;
    /** Run the entry at `index` (of the filtered list). */
    onchoose: (index: number) => void;
    onclose: () => void;
  }
  let { menu, onchoose, onclose }: Props = $props();
  let input: HTMLInputElement;

  $effect(() => input.focus());

  function onkeydown(e: KeyboardEvent) {
    const ctrlOnly = e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey;
    switch (e.key) {
      case "ArrowDown":
        menu.move(1);
        break;
      case "ArrowUp":
        menu.move(-1);
        break;
      case "Enter":
        onchoose(menu.selected);
        break;
      case "Escape":
        onclose();
        break;
      case "Tab":
        break;
      default:
        // Ctrl+K again closes, as it opened.
        if (ctrlOnly && e.key.toLowerCase() === "k") {
          onclose();
          break;
        }
        return;
    }
    e.preventDefault();
    // Esc must not also reach the launcher (which would clear the search).
    e.stopPropagation();
  }
</script>

<div class="menu" role="dialog" aria-label="Actions">
  <input
    bind:this={input}
    value={menu.filter}
    oninput={(e) => menu.setFilter(e.currentTarget.value)}
    {onkeydown}
    onblur={onclose}
    placeholder="Search actions…"
    autocomplete="off"
    spellcheck="false"
    role="combobox"
    aria-expanded="true"
    aria-controls="actions"
    aria-activedescendant={menu.entries.length > 0 ? `action-${menu.selected}` : undefined}
  />
  <ul id="actions" role="listbox" aria-label="Actions">
    {#each menu.entries as entry, i (entry.action)}
      <!-- Keys go to the filter box; entries only take the mouse. The
           mousedown keeps focus in the box so it doesn't close first. -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <li
        id={`action-${i}`}
        role="option"
        aria-selected={i === menu.selected}
        class:selected={i === menu.selected}
        onmousedown={(e) => e.preventDefault()}
        onmousemove={() => (menu.selected = i)}
        onclick={() => onchoose(i)}
      >
        <span class="label">{entry.label}</span>
        {#if entry.key}
          <kbd>{entry.key}</kbd>
        {/if}
      </li>
    {:else}
      <li class="empty">No matching action</li>
    {/each}
  </ul>
</div>

<style>
  .menu {
    position: absolute;
    right: var(--space-3);
    bottom: var(--space-2);
    width: var(--menu-width);
    display: flex;
    flex-direction: column;
    padding: var(--space-1);
    border: 1px solid var(--popover-border);
    border-radius: var(--radius-row);
    background: var(--popover);
    box-shadow: var(--popover-shadow);
  }
  input {
    padding: var(--space-2) var(--space-3);
    border: none;
    border-bottom: 1px solid var(--hairline);
    outline: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--font-size-title);
  }
  input::placeholder {
    color: var(--text-faint);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: var(--space-1) 0 0;
    max-height: calc(var(--row-height) * 6);
    overflow-y: auto;
    scrollbar-width: none;
  }
  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    height: var(--menu-row-height);
    padding: 0 var(--space-3);
    border: 1px solid transparent;
    border-radius: var(--radius-keycap);
    font-size: var(--font-size-title);
    cursor: default;
  }
  li.selected {
    background: var(--selection-sheen), var(--selection);
    border-color: var(--selection-border);
  }
  li.empty {
    color: var(--text-faint);
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  kbd {
    flex: none;
    padding: 0 var(--space-1);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    font: inherit;
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
</style>
