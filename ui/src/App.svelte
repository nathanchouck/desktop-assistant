<script lang="ts">
  import { onMount } from "svelte";
  import SearchBar from "./components/SearchBar.svelte";
  import ResultList from "./components/ResultList.svelte";
  import Footer from "./components/Footer.svelte";
  import { commands, events } from "./lib/bindings";
  import { intentFor } from "./lib/keys";
  import { SearchStore } from "./lib/stores/search.svelte";

  const search = new SearchStore();
  let bar: SearchBar;
  let content: HTMLDivElement;

  function onkeydown(e: KeyboardEvent) {
    const intent = intentFor(e);
    if (!intent) return;
    e.preventDefault();
    switch (intent.type) {
      case "move":
        search.move(intent.delta);
        break;
      case "activate":
        void search.activate(intent.index, intent.action);
        break;
      case "escape":
        // First Esc clears the query; the next one hides the launcher.
        if (search.query) search.clear();
        else void commands.launcherHide();
        break;
    }
  }

  onMount(() => {
    // Fit the window to the content. The results area has a fixed height,
    // so this only fires when switching between bar-only and expanded.
    let lastHeight = 0;
    const observer = new ResizeObserver(([entry]) => {
      const height = Math.ceil(entry?.borderBoxSize[0]?.blockSize ?? 0);
      if (height > 0 && height !== lastHeight) {
        lastHeight = height;
        void commands.launcherSetHeight(height);
      }
    });
    observer.observe(content);

    const unlisten = events.launcherShown.listen(() => bar.focusAndSelect());

    return () => {
      observer.disconnect();
      void unlisten.then((stop) => stop());
    };
  });
</script>

<main>
  <div class="content" bind:this={content}>
    <SearchBar
      bind:this={bar}
      value={search.query}
      activeIndex={search.items.length > 0 ? search.selected : null}
      oninput={(v) => search.setQuery(v)}
      {onkeydown}
    />
    {#if search.expanded}
      <ResultList
        items={search.items}
        selected={search.selected}
        onselect={(i) => (search.selected = i)}
        onactivate={(i) => search.activate(i)}
      />
      <Footer notice={search.notice} actions={search.current?.actions ?? []} />
    {/if}
  </div>
</main>

<style>
  main {
    height: 100%;
    background: var(--surface-sheen), var(--surface);
    /* Inner highlight: bright along the top edge, faint along the bottom.
     * Windows 11 rounds the corners and draws the outer shadow itself. */
    box-shadow:
      inset 0 1px 0 var(--highlight-top),
      inset 0 -1px 0 var(--highlight-bottom);
    overflow: hidden;
  }
  .content {
    display: flex;
    flex-direction: column;
  }
</style>
