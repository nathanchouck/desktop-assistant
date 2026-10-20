<script lang="ts">
  import SearchBar from "./components/SearchBar.svelte";
  import ResultList from "./components/ResultList.svelte";
  import { SearchStore } from "./lib/stores/search.svelte";

  const search = new SearchStore();

  function onkeydown(e: KeyboardEvent) {
    switch (e.key) {
      case "ArrowDown":
        search.move(1);
        break;
      case "ArrowUp":
        search.move(-1);
        break;
      case "Enter":
        void search.activate();
        break;
      default:
        return;
    }
    e.preventDefault();
  }
</script>

<main class="glass">
  <SearchBar
    value={search.query}
    activeIndex={search.items.length > 0 ? search.selected : null}
    oninput={(v) => search.setQuery(v)} {onkeydown} />
  {#if search.items.length > 0}
    <div class="divider"></div>
    <ResultList
      items={search.items}
      selected={search.selected}
      onselect={(i) => (search.selected = i)}
      onactivate={(i) => search.activate(i)}
    />
  {/if}
  {#if search.notice}
    <p class="notice">{search.notice}</p>
  {/if}
</main>

<style>
  .glass {
    height: 100%;
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-window);
    background: var(--glass-tint);
    backdrop-filter: blur(var(--glass-blur)) saturate(1.6);
    border: 1px solid transparent;
    background-clip: padding-box;
    /* Inner highlight: bright at the top edge, fading downward. */
    box-shadow:
      inset 0 1px 0 var(--glass-highlight-top),
      inset 0 -1px 0 var(--glass-highlight-bottom),
      var(--glass-shadow);
    overflow: hidden;
  }
  .divider {
    height: 1px;
    margin: 0 var(--space-4) var(--space-2);
    background: var(--glass-highlight-bottom);
  }
  .notice {
    margin: auto var(--space-5) var(--space-3);
    font-size: var(--font-size-sub);
    color: var(--text-muted);
  }
</style>
