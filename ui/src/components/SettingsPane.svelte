<script lang="ts">
  import { launcher } from "../lib/stores/launcher.svelte";
  import type { SettingsStore } from "../lib/stores/settings.svelte";
  import AdvancedSection from "./settings/AdvancedSection.svelte";
  import AiCommandsSection from "./settings/AiCommandsSection.svelte";
  import AppearanceSection from "./settings/AppearanceSection.svelte";
  import CalculatorSection from "./settings/CalculatorSection.svelte";
  import ChatGptSection from "./settings/ChatGptSection.svelte";
  import ClaudeSection from "./settings/ClaudeSection.svelte";
  import ClipboardSection from "./settings/ClipboardSection.svelte";
  import ColorSection from "./settings/ColorSection.svelte";
  import EmojiSection from "./settings/EmojiSection.svelte";
  import GeneralSection from "./settings/GeneralSection.svelte";
  import GoogleSection from "./settings/GoogleSection.svelte";
  import HistorySection from "./settings/HistorySection.svelte";
  import IntegrationsSection from "./settings/IntegrationsSection.svelte";
  import QuicklinksSection from "./settings/QuicklinksSection.svelte";
  import ScriptsSection from "./settings/ScriptsSection.svelte";
  import SearchSection from "./settings/SearchSection.svelte";
  import SnippetsSection from "./settings/SnippetsSection.svelte";
  import SystemSection from "./settings/SystemSection.svelte";
  import UpdatesSection from "./settings/UpdatesSection.svelte";

  interface Props {
    store: SettingsStore;
    onclose: () => void;
  }
  let { store, onclose }: Props = $props();
  let pane: HTMLDivElement;

  /** Focus the menu so the keyboard works in it. */
  export function focus(): void {
    pane.focus();
  }

  // Keep the status lines fresh while the menu is on screen. The menu
  // stays open in the hidden window, so stop then (and look again on open).
  $effect(() => {
    if (!launcher.visible) return;
    void store.refreshStatus();
    const timer = setInterval(() => void store.refreshStatus(), 2000);
    return () => clearInterval(timer);
  });

  function onkeydown(e: KeyboardEvent) {
    // A control that uses Esc itself (the shortcut recorder) stops it first.
    if (e.key === "Escape") {
      e.preventDefault();
      onclose();
    }
  }
</script>

<!-- Esc is handled for the whole menu. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="pane" bind:this={pane} tabindex="-1" {onkeydown} role="region" aria-label="Settings">
  {#if store.error}
    <p class="error" role="alert">{store.error}</p>
  {/if}
  <!-- One section per area; each feature fills in its own file. -->
  <GeneralSection {store} />
  <ClaudeSection {store} />
  <ChatGptSection {store} />
  <HistorySection {store} />
  <AiCommandsSection {store} />
  <IntegrationsSection {store} />
  <SearchSection {store} />
  <CalculatorSection {store} />
  <ClipboardSection {store} />
  <SnippetsSection {store} />
  <QuicklinksSection {store} />
  <EmojiSection {store} />
  <ScriptsSection {store} />
  <SystemSection {store} />
  <ColorSection {store} />
  <GoogleSection {store} />
  <AppearanceSection {store} />
  <UpdatesSection {store} />
  <AdvancedSection {store} />
</div>

<style>
  .pane {
    height: calc(var(--row-height) * var(--results-rows) + var(--space-2));
    padding: 0 var(--space-5) var(--space-4);
    overflow-y: auto;
    scrollbar-width: thin;
    outline: none;
    font-size: var(--font-size-title);
  }
  .error {
    margin: var(--space-2) 0 0;
    font-size: var(--font-size-sub);
    color: var(--danger);
  }
</style>
