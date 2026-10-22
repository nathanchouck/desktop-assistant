<script lang="ts">
  import { onMount } from "svelte";
  import SearchBar from "./components/SearchBar.svelte";
  import ResultList from "./components/ResultList.svelte";
  import ChatPane from "./components/ChatPane.svelte";
  import SettingsPane from "./components/SettingsPane.svelte";
  import Footer from "./components/Footer.svelte";
  import { commands, events, type IndexStatus, type ProviderId } from "./lib/bindings";
  import { answeringLabel, forcedProvider, looksLikeAQuestion, PROVIDER_NAMES } from "./lib/chat";
  import { indexNotice } from "./lib/display";
  import { chatHints, chatIntentFor, intentFor, searchHints } from "./lib/keys";
  import { ChatStore } from "./lib/stores/chat.svelte";
  import { SearchStore } from "./lib/stores/search.svelte";
  import { SettingsStore } from "./lib/stores/settings.svelte";
  import { ToastStore } from "./lib/stores/toast.svelte";
  import { applyTheme, themeFor } from "./lib/theme";
  import { tick } from "svelte";

  /** A finished chat is put away if the launcher stayed hidden this long. */
  const CHAT_RESET_MS = 5 * 60 * 1000;

  const chat = new ChatStore();
  const settings = new SettingsStore();
  /** The AI that Tab and the "Ask" row use (loaded at start, kept current by the menu). */
  const defaultProvider = (): ProviderId => settings.settings?.ai.default_provider ?? "claude";
  const search = new SearchStore({
    onAsk: (text, provider) => {
      search.clear();
      void chat.ask(text, provider);
    },
    onSettings: () => void openSettings(),
    defaultProvider,
  });
  const toasts = new ToastStore();
  let bar: SearchBar | undefined = $state();
  let pane: ChatPane | undefined = $state();
  let settingsPane: SettingsPane | undefined = $state();
  /** The settings menu is showing (over search or a chat). */
  let settingsOpen = $state(false);
  let content: HTMLDivElement;
  let hiddenAt = 0;
  let indexStatus = $state<IndexStatus | null>(null);
  /** The AI was started early for the query being typed. */
  let warmed = false;

  // A question is being typed: start the AI it's for now so it answers sooner.
  $effect(() => {
    if (!search.query.trim()) warmed = false;
    else if (!warmed && !chat.active && looksLikeAQuestion(search.query, search.items)) {
      warmed = true;
      void commands.aiWarm(forcedProvider(search.query)?.provider ?? null);
    }
  });

  // The look from Settings → Appearance, applied as soon as it changes.
  $effect(() => {
    const appearance = settings.settings?.appearance;
    if (appearance) applyTheme(document.documentElement, themeFor(appearance));
  });

  const expanded = $derived(settingsOpen || chat.active || search.expanded || toasts.current !== null);
  const notice = $derived(
    settingsOpen
      ? "Changes are saved as you make them"
      : chat.active
        ? answeringLabel(chat.provider ?? defaultProvider(), chat.transcript.model)
        : (search.notice ?? indexNotice(indexStatus)),
  );
  const hints = $derived(
    settingsOpen
      ? ([["Esc", "Close"]] as [string, string][])
      : chat.active
        ? chatHints({ running: chat.transcript.running, pending: chat.pending !== undefined })
        : searchHints(search.current?.actions ?? [], PROVIDER_NAMES[search.otherProvider()]),
  );

  async function openSettings() {
    await settings.load();
    settingsOpen = true;
    await tick();
    settingsPane?.focus();
  }

  async function closeSettings() {
    settingsOpen = false;
    await tick();
    bar?.focusAndSelect();
  }

  function onSearchKey(e: KeyboardEvent) {
    const intent = intentFor(e);
    if (!intent) return;
    e.preventDefault();
    switch (intent.type) {
      case "move":
        search.move(intent.delta);
        break;
      case "activate":
        // "c <question>" / "g <question>" + Enter skips the results and
        // asks that AI.
        if (intent.index === undefined && !intent.actions && forcedProvider(search.query)) {
          search.ask();
        } else {
          void search.activate(intent.index, intent.actions);
        }
        break;
      case "ask":
        if (intent.other) search.askOther();
        else search.ask();
        break;
      case "settings":
        void openSettings();
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
    if (settingsOpen) settingsPane?.focus();
    else bar?.focusAndSelect();
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
      events.showSettings.listen(() => void openSettings()),
      // Another PC changed the synced settings (or the file was edited).
      events.settingsChanged.listen((e) => settings.changed(e.payload.settings)),
      events.toast.listen((e) => toasts.show(e.payload)),
      // A color was picked: list recent colors, the new one first.
      events.colorPicked.listen(() => {
        if (settingsOpen || chat.active) return;
        void search.setQuery("colors").then(() => bar?.focusAndSelect());
      }),
    ];
    void commands.fileIndexStatus().then((s) => (indexStatus = s));
    // Which AI is the default (for Tab, Shift+Tab and the chat footer).
    void settings.load();

    return () => {
      observer.disconnect();
      window.removeEventListener("blur", onBlur);
      for (const u of unlisten) void u.then((stop) => stop());
    };
  });
</script>

<main>
  <div class="content" bind:this={content}>
    {#if settingsOpen}
      <div class="settings-head">
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
          <line x1="4" y1="7" x2="20" y2="7" />
          <circle cx="9" cy="7" r="2.4" />
          <line x1="4" y1="17" x2="20" y2="17" />
          <circle cx="15" cy="17" r="2.4" />
        </svg>
        <span>Settings</span>
        <button type="button" onclick={() => void closeSettings()}>Done</button>
      </div>
    {:else}
      <!-- One input for both modes: a search, or a follow-up while chatting. -->
      <SearchBar
        bind:this={bar}
        chat={chat.active}
        value={chat.active ? chat.draft : search.query}
        activeIndex={!chat.active && search.items.length > 0 ? search.selected : null}
        oninput={(v) => (chat.active ? (chat.draft = v) : search.setQuery(v))}
        onkeydown={(e) => (chat.active ? onChatKey(e) : onSearchKey(e))}
      />
    {/if}
    {#if expanded}
      {#if settingsOpen}
        <SettingsPane bind:this={settingsPane} store={settings} onclose={() => void closeSettings()} />
      {:else if chat.active}
        <ChatPane bind:this={pane} transcript={chat.transcript} onanswer={(allow) => chat.answer(allow)} />
      {:else}
        <ResultList
          items={search.items}
          selected={search.selected}
          onselect={(i) => (search.selected = i)}
          onactivate={(i) => search.activate(i)}
        />
      {/if}
      <Footer {notice} {hints} toast={toasts.current} ondismiss={() => toasts.dismiss()} />
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
  .settings-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5);
    font-size: var(--font-size-input);
  }
  .settings-head span {
    flex: 1;
  }
  .settings-head .icon {
    width: 22px;
    height: 22px;
    flex: none;
    fill: var(--text-muted);
    stroke: var(--text-muted);
    stroke-width: 2;
    stroke-linecap: round;
  }
  .settings-head button {
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    color: var(--text);
    font: inherit;
    font-size: var(--font-size-sub);
  }
</style>
