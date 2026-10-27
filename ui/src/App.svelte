<script lang="ts">
  import { onMount } from "svelte";
  import SearchBar from "./components/SearchBar.svelte";
  import ResultList from "./components/ResultList.svelte";
  import ActionMenu from "./components/ActionMenu.svelte";
  import ChatPane from "./components/ChatPane.svelte";
  import ComparePane from "./components/ComparePane.svelte";
  import ComposePane from "./components/ComposePane.svelte";
  import SettingsPane from "./components/SettingsPane.svelte";
  import Footer from "./components/Footer.svelte";
  import { commands, events, type Attachment, type IndexStatus, type ProviderId } from "./lib/bindings";
  import { answeringLabel, forcedProvider, looksLikeAQuestion, otherProvider, PROVIDER_NAMES } from "./lib/chat";
  import { COMPARE_ROW_ID, compareNotice, comparePrefix } from "./lib/compare";
  import { composedQuestion, DEFAULT_QUESTION } from "./lib/compose";
  import { indexNotice } from "./lib/display";
  import {
    chatHints,
    chatIntentFor,
    compareHints,
    compareIntentFor,
    composeHints,
    composeIntentFor,
    intentFor,
    searchHints,
  } from "./lib/keys";
  import { ActionMenuStore } from "./lib/stores/actionMenu.svelte";
  import { ChatStore } from "./lib/stores/chat.svelte";
  import { CompareStore } from "./lib/stores/compare.svelte";
  import { ComposeStore } from "./lib/stores/compose.svelte";
  import { launcher } from "./lib/stores/launcher.svelte";
  import { SearchStore } from "./lib/stores/search.svelte";
  import { SettingsStore } from "./lib/stores/settings.svelte";
  import { ToastStore } from "./lib/stores/toast.svelte";
  import { applyTheme, themeFor } from "./lib/theme";
  import { tick } from "svelte";

  /** A finished chat is put away if the launcher stayed hidden this long. */
  const CHAT_RESET_MS = 5 * 60 * 1000;

  const chat = new ChatStore();
  /** Claude and ChatGPT answering the same question side by side. */
  const compare = new CompareStore();
  /** A question with attachments (the selection, a file, a snip) being typed. */
  const compose = new ComposeStore();
  const settings = new SettingsStore();
  /** The AI that Tab and the "Ask" row use (loaded at start, kept current by the menu). */
  const defaultProvider = (): ProviderId => settings.settings?.ai.default_provider ?? "claude";
  const search = new SearchStore({
    onAsk: (text, provider) => {
      search.clear();
      void chat.ask(text, provider);
    },
    onSettings: () => void openSettings(),
    onOpenChat: (saved) => chat.reopen(saved),
    onCompose: (attachments) => void startCompose(attachments),
    onCompare: (text) => {
      search.clear();
      void compare.start(text);
    },
    defaultProvider,
  });
  const toasts = new ToastStore();
  /** Ctrl+K: the selected result's actions. */
  const menu = new ActionMenuStore();
  /** Shows why copying or pasting an answer failed. */
  const showError = (message: string | null) => {
    if (message) toasts.show({ level: "error", message });
  };
  let bar: SearchBar | undefined = $state();
  let pane: ChatPane | undefined = $state();
  let comparePane: ComparePane | undefined = $state();
  let settingsPane: SettingsPane | undefined = $state();
  /** The settings menu is showing (over search or a chat). */
  let settingsOpen = $state(false);
  let content: HTMLDivElement;
  let hiddenAt = 0;
  let indexStatus = $state<IndexStatus | null>(null);
  /** The AI was started early for the query being typed. */
  let warmed = false;
  /** The same for a question with attachments. */
  let composeWarmed = false;
  /** The window is wide for two answers side by side. */
  let wide = false;
  /** A chat of either kind is showing; the input takes its follow-up. */
  const chatting = $derived(chat.active || compare.active);
  /** Attachments, no chat: the input takes the question about them. */
  const composing = $derived(compose.active && !chatting);
  /** The AI that Enter asks while composing (a `c `/`g ` prefix picks one). */
  const composeAi = $derived(forcedProvider(compose.draft)?.provider ?? defaultProvider());

  // A question is being typed: start the AI it's for now so it answers sooner.
  $effect(() => {
    if (!search.query.trim()) warmed = false;
    else if (!warmed && !chatting && looksLikeAQuestion(search.query, search.items)) {
      warmed = true;
      if (comparePrefix(search.query) !== null) void commands.aiCompareWarm();
      else void commands.aiWarm(forcedProvider(search.query)?.provider ?? null);
    }
  });

  // Two answers side by side get a wider window, once as the view opens
  // and once as it closes (like the two heights, never while typing).
  $effect(() => {
    if (compare.active === wide) return;
    wide = compare.active;
    void commands.launcherSetWide(wide);
  });

  // Attachments came in: start the AI now, the question is on its way.
  $effect(() => {
    if (!composing) composeWarmed = false;
    else if (!composeWarmed) {
      composeWarmed = true;
      void commands.aiWarm(null);
    }
  });

  // The look from Settings → Appearance, applied as soon as it changes.
  $effect(() => {
    const appearance = settings.settings?.appearance;
    if (appearance) applyTheme(document.documentElement, themeFor(appearance));
  });

  const expanded = $derived(settingsOpen || chatting || composing || search.expanded || toasts.current !== null);
  const notice = $derived(
    settingsOpen
      ? "Changes are saved as you make them"
      : chat.active
        ? answeringLabel(chat.provider ?? defaultProvider(), chat.transcript.model)
        : compare.active
          ? compareNotice(compare.sides)
          : composing
            ? null
            : (search.notice ?? indexNotice(indexStatus)),
  );
  /** Whose approval Ctrl+Y answers while comparing. */
  const comparePending = $derived.by(() => {
    const side = compare.pending ? compare.sides[compare.pending.side] : undefined;
    return side ? PROVIDER_NAMES[side.provider] : null;
  });
  const hints = $derived(
    settingsOpen
      ? ([["Esc", "Close"]] as [string, string][])
      : menu.open
        ? ([
            ["↵", "Run"],
            ["Esc", "Back"],
          ] as [string, string][])
      : chat.active
        ? chatHints({
            running: chat.transcript.running,
            pending: chat.pending !== undefined,
            answer: chat.finishedAnswer !== null,
          })
        : compare.active
          ? compareHints({
              running: compare.running,
              pending: comparePending,
              names: compare.sides.map((s) => PROVIDER_NAMES[s.provider]),
            })
          : composing
            ? composeHints(PROVIDER_NAMES[composeAi], PROVIDER_NAMES[otherProvider(composeAi)], !compose.draft, true)
            : searchHints(
                search.current?.actions ?? [],
                PROVIDER_NAMES[search.otherProvider()],
                search.current?.id === COMPARE_ROW_ID ? "selected" : search.compareOffered ? "offered" : "no",
              ),
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
        } else if (intent.index === undefined && !intent.actions && comparePrefix(search.query) !== null) {
          // "both <question>" + Enter: both AIs, even before the row shows.
          search.compare();
        } else {
          void search.activate(intent.index, intent.actions);
        }
        break;
      case "ask":
        if (intent.other) search.askOther();
        else search.ask();
        break;
      case "compare":
        search.compare();
        break;
      case "settings":
        void openSettings();
        break;
      case "menu":
        if (search.current) menu.show(search.current);
        break;
      case "escape":
        // First Esc clears the query; the next one hides the launcher.
        if (search.query) search.clear();
        else void commands.launcherHide();
        break;
    }
  }

  /**
   * Starts a question with `attachments`: the launcher shows them as chips
   * and the input takes the question. A finished chat is put away first
   * (it's in history); a running one keeps going, and they go with its
   * next follow-up.
   */
  async function startCompose(attachments: Attachment[]) {
    if (attachments.length === 0) return;
    settingsOpen = false;
    if (chat.active && !chat.transcript.running && !chat.pending) await chat.close();
    if (compare.active && !compare.running && !compare.pending) await compare.close();
    search.clear();
    compose.add(attachments);
    await tick();
    bar?.focus();
  }

  /** Asks about the attachments: the default AI (or a prefix's), or the other one. */
  function askComposed(other: boolean) {
    const { text, provider } = composedQuestion(compose.draft);
    const target = other ? otherProvider(provider ?? defaultProvider()) : provider;
    void chat.ask(text, target, compose.take());
  }

  function onComposeKey(e: KeyboardEvent) {
    const intent = composeIntentFor(e, !compose.draft);
    if (!intent) return;
    e.preventDefault();
    switch (intent.type) {
      case "ask":
        askComposed(intent.other === true);
        break;
      case "compare":
        // Both AIs get the same attachments (a `both `, `c ` or `g ` prefix is dropped).
        void compare.start(composedQuestion(comparePrefix(compose.draft) ?? compose.draft).text, compose.take());
        break;
      case "remove_last":
        compose.removeLast();
        break;
      case "escape":
        compose.clear();
        break;
    }
  }

  /** The mic button: Windows voice typing into the box. */
  async function dictate() {
    bar?.focus();
    const res = await commands.aiDictate();
    if (res.status === "error") showError(res.error.message);
  }

  /** Runs the action menu's entry `index` on the row it was opened for. */
  function chooseAction(index: number) {
    const item = menu.item;
    const action = menu.choose(index);
    bar?.focus();
    if (!item || !action) return;
    // Late results may have moved the row since the menu opened.
    const row = search.items.findIndex((i) => i.id === item.id);
    if (row >= 0) void search.activate(row, [action]);
  }

  function closeMenu() {
    if (!menu.open) return;
    menu.close();
    bar?.focus();
  }

  function onChatKey(e: KeyboardEvent) {
    // Backspace in an empty box drops the last attachment.
    if (compose.active && composeIntentFor(e, !chat.draft)?.type === "remove_last") {
      e.preventDefault();
      compose.removeLast();
      return;
    }
    const intent = chatIntentFor(e);
    if (!intent) return;
    e.preventDefault();
    switch (intent.type) {
      case "send":
        if (chat.transcript.running) break;
        if (compose.active && !chat.draft.trim()) chat.draft = DEFAULT_QUESTION;
        void chat.followUp(compose.active ? compose.take() : []);
        break;
      case "approve":
        void chat.answer(intent.allow);
        break;
      case "stop":
        void chat.stop();
        break;
      case "paste_answer":
        void chat.pasteAnswer().then(showError);
        break;
      case "copy_answer":
        void chat.copyAnswer().then(showError);
        break;
      case "scroll":
        pane?.scroll(intent.delta, intent.pages);
        break;
      case "escape":
        // Each Esc backs out one step: deny, stop, clear the draft, drop
        // the attachments, close.
        if (chat.pending) void chat.answer(false);
        else if (chat.transcript.running) void chat.stop();
        else if (chat.draft) chat.draft = "";
        else if (compose.active) compose.clear();
        else void chat.close();
        break;
    }
  }

  function onCompareKey(e: KeyboardEvent) {
    // Backspace in an empty box drops the last attachment.
    if (compose.active && composeIntentFor(e, !compare.draft)?.type === "remove_last") {
      e.preventDefault();
      compose.removeLast();
      return;
    }
    const intent = compareIntentFor(e);
    if (!intent) return;
    e.preventDefault();
    switch (intent.type) {
      case "send":
        if (compare.running) break;
        if (compose.active && !compare.draft.trim()) compare.draft = DEFAULT_QUESTION;
        void compare.followUp(compose.active ? compose.take() : []);
        break;
      case "approve":
        void compare.answer(intent.allow);
        break;
      case "stop":
        void compare.stop();
        break;
      case "keep":
        keepSide(intent.side);
        break;
      case "scroll":
        comparePane?.scroll(intent.delta, intent.pages);
        break;
      case "escape":
        // Each Esc backs out one step: deny, stop both, clear the draft,
        // drop the attachments, close.
        if (compare.pending) void compare.answer(false);
        else if (compare.running) void compare.stop();
        else if (compare.draft) compare.draft = "";
        else if (compose.active) compose.clear();
        else void compare.close();
        break;
    }
  }

  /** Ctrl+1 / Ctrl+2: carry on with that AI alone, as a normal chat. */
  function keepSide(index: number) {
    const kept = compare.release(index);
    if (kept) void chat.continueFrom(kept.side, kept.draft);
  }

  function onHidden() {
    launcher.visible = false;
    toasts.hidden();
  }

  function onShown() {
    launcher.visible = true;
    toasts.shown();
    const idle = Date.now() - hiddenAt > CHAT_RESET_MS;
    if (chat.active && idle && !chat.transcript.running && !chat.pending) void chat.close();
    if (compare.active && idle && !compare.running && !compare.pending) void compare.close();
    if (idle) compose.clear();
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
    // Release builds start hidden in the tray; toasts wait for the first open.
    if (!document.hasFocus()) onHidden();
    const unlisten = [
      events.launcherShown.listen(onShown),
      events.launcherHidden.listen(onHidden),
      events.aiStream.listen((e) => {
        chat.onEvent(e.payload);
        compare.onEvent(e.payload);
      }),
      events.fileIndexChanged.listen((e) => (indexStatus = e.payload.status)),
      events.searchUpdated.listen((e) => search.onUpdate(e.payload)),
      events.showSettings.listen(() => void openSettings()),
      // Another PC changed the synced settings (or the file was edited).
      events.settingsChanged.listen((e) => settings.changed(e.payload.settings)),
      events.toast.listen((e) => toasts.show(e.payload)),
      // The selection shortcut or a snip: ask about what came with it.
      events.composeRequested.listen((e) => void startCompose(e.payload.attachments)),
      // A color was picked: list recent colors, the new one first.
      events.colorPicked.listen(() => {
        if (settingsOpen || chatting) return;
        void search.setQuery("colors").then(() => bar?.focusAndSelect());
      }),
    ];
    void commands.fileIndexStatus().then((s) => (indexStatus = s));
    // Problems from before the UI listened (startup), oldest first.
    void commands.toastRecent().then((recent) => {
      for (const { toast, age_ms } of recent) toasts.show(toast, Date.now() - age_ms);
    });
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
        chat={chatting}
        attachments={compose.attachments}
        value={chat.active ? chat.draft : compare.active ? compare.draft : composing ? compose.draft : search.query}
        activeIndex={!chatting && !composing && search.items.length > 0 ? search.selected : null}
        oninput={(v) =>
          chat.active
            ? (chat.draft = v)
            : compare.active
              ? (compare.draft = v)
              : composing
                ? (compose.draft = v)
                : search.setQuery(v)}
        onkeydown={(e) =>
          chat.active
            ? onChatKey(e)
            : compare.active
              ? onCompareKey(e)
              : composing
                ? onComposeKey(e)
                : onSearchKey(e)}
        onremove={(i) => compose.remove(i)}
        ondictate={() => void dictate()}
      />
    {/if}
    {#if expanded}
      {#if settingsOpen}
        <SettingsPane bind:this={settingsPane} store={settings} onclose={() => void closeSettings()} />
      {:else if chat.active}
        <ChatPane bind:this={pane} transcript={chat.transcript} onanswer={(allow) => chat.answer(allow)} />
      {:else if compare.active}
        <ComparePane bind:this={comparePane} store={compare} onanswer={(side, allow) => compare.answer(allow, side)} />
      {:else if composing}
        <ComposePane attachments={compose.attachments} ai={PROVIDER_NAMES[composeAi]} />
      {:else}
        <div class="results">
          <ResultList
            items={search.items}
            selected={search.selected}
            onselect={(i) => (search.selected = i)}
            onactivate={(i) => search.activate(i)}
          />
          {#if menu.open}
            <ActionMenu {menu} onchoose={chooseAction} onclose={closeMenu} />
          {/if}
        </div>
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
  .results {
    position: relative;
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
