<script lang="ts">
  import { onMount } from "svelte";
  import SearchBar from "./components/SearchBar.svelte";
  import ResultList from "./components/ResultList.svelte";
  import ChatPane from "./components/ChatPane.svelte";
  import Footer from "./components/Footer.svelte";
  import { commands, events, type IndexStatus } from "./lib/bindings";
  import { hasClaudePrefix, looksLikeAQuestion, modelLabel, stripClaudePrefix } from "./lib/chat";
  import { indexNotice } from "./lib/display";
  import { chatHints, chatIntentFor, intentFor, searchHints } from "./lib/keys";
  import { ChatStore } from "./lib/stores/chat.svelte";
  import { SearchStore } from "./lib/stores/search.svelte";

  /** A finished chat is put away if the launcher stayed hidden this long. */
  const CHAT_RESET_MS = 5 * 60 * 1000;

  const chat = new ChatStore();
  const search = new SearchStore((text) => {
    search.clear();
    void chat.ask(text);
  });
  let bar: SearchBar;
  let pane: ChatPane | undefined = $state();
  let content: HTMLDivElement;
  let hiddenAt = 0;
  let indexStatus = $state<IndexStatus | null>(null);
  /** Claude Code was started early for the query being typed. */
  let warmed = false;

  // A question is being typed: start Claude Code now so it answers sooner.
  $effect(() => {
    if (!search.query.trim()) warmed = false;
    else if (!warmed && !chat.active && looksLikeAQuestion(search.query, search.items)) {
      warmed = true;
      void commands.aiWarm();
    }
  });

  const expanded = $derived(chat.active || search.expanded);
  const notice = $derived(
    chat.active
      ? `Claude${chat.transcript.model ? ` · ${modelLabel(chat.transcript.model)}` : ""}`
      : (search.notice ?? indexNotice(indexStatus)),
  );
  const hints = $derived(
    chat.active
      ? chatHints({ running: chat.transcript.running, pending: chat.pending !== undefined })
      : searchHints(search.current?.actions ?? []),
  );

  function onSearchKey(e: KeyboardEvent) {
    const intent = intentFor(e);
    if (!intent) return;
    e.preventDefault();
    switch (intent.type) {
      case "move":
        search.move(intent.delta);
        break;
      case "activate":
        // "c <question>" + Enter skips the results and asks Claude.
        if (intent.index === undefined && !intent.action && hasClaudePrefix(search.query)) {
          search.ask(stripClaudePrefix(search.query));
        } else {
          void search.activate(intent.index, intent.action);
        }
        break;
      case "ask":
        search.ask(hasClaudePrefix(search.query) ? stripClaudePrefix(search.query) : search.query);
        break;
      case "escape":
        // First Esc clears the query; the next one hides the launcher.
        if (search.query) search.clear();
        else void commands.launcherHide();
        break;
    }
  }

  function onChatKey(e: KeyboardEvent) {
    const intent = chatIntentFor(e);
    if (!intent) return;
    e.preventDefault();
    switch (intent.type) {
      case "send":
        void chat.followUp();
        break;
      case "approve":
        void chat.answer(intent.allow);
        break;
      case "stop":
        void chat.stop();
        break;
      case "scroll":
        pane?.scroll(intent.delta, intent.pages);
        break;
      case "escape":
        // Each Esc backs out one step: deny, stop, clear the draft, close.
        if (chat.pending) void chat.answer(false);
        else if (chat.transcript.running) void chat.stop();
        else if (chat.draft) chat.draft = "";
        else void chat.close();
        break;
    }
  }

  function onShown() {
    const idle = Date.now() - hiddenAt > CHAT_RESET_MS;
    if (chat.active && idle && !chat.transcript.running && !chat.pending) void chat.close();
    bar.focusAndSelect();
    void commands.fileIndexStatus().then((s) => (indexStatus = s));
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

    const onBlur = () => (hiddenAt = Date.now());
    window.addEventListener("blur", onBlur);
    const unlisten = [
      events.launcherShown.listen(onShown),
      events.aiStream.listen((e) => chat.onEvent(e.payload)),
      events.fileIndexChanged.listen((e) => (indexStatus = e.payload.status)),
      events.searchUpdated.listen((e) => search.onUpdate(e.payload)),
    ];
    void commands.fileIndexStatus().then((s) => (indexStatus = s));

    return () => {
      observer.disconnect();
      window.removeEventListener("blur", onBlur);
      for (const u of unlisten) void u.then((stop) => stop());
    };
  });
</script>

<main>
  <div class="content" bind:this={content}>
    <!-- One input for both modes: a search, or a follow-up while chatting. -->
    <SearchBar
      bind:this={bar}
      chat={chat.active}
      value={chat.active ? chat.draft : search.query}
      activeIndex={!chat.active && search.items.length > 0 ? search.selected : null}
      oninput={(v) => (chat.active ? (chat.draft = v) : search.setQuery(v))}
      onkeydown={(e) => (chat.active ? onChatKey(e) : onSearchKey(e))}
    />
    {#if expanded}
      {#if chat.active}
        <ChatPane bind:this={pane} transcript={chat.transcript} onanswer={(allow) => chat.answer(allow)} />
      {:else}
        <ResultList
          items={search.items}
          selected={search.selected}
          onselect={(i) => (search.selected = i)}
          onactivate={(i) => search.activate(i)}
        />
      {/if}
      <Footer {notice} {hints} />
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
