<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import type { ItemKind } from "../lib/bindings";

  interface Props {
    kind: ItemKind;
    /** Key for the `icon` protocol; falls back to a glyph if missing or broken. */
    icon: string | null;
  }
  let { kind, icon }: Props = $props();
  let broken = $state(false);

  // A new icon key (row reused for another item) gets a fresh chance.
  $effect(() => {
    void icon;
    broken = false;
  });
</script>

<span class="icon" aria-hidden="true">
  {#if icon && !broken}
    <img src={convertFileSrc(icon, "icon")} alt="" draggable="false" onerror={() => (broken = true)} />
  {:else}
    <svg viewBox="0 0 24 24" class:ai={kind === "ask_ai"}>
      {#if kind === "ask_ai"}
        <path d="M12 3l1.8 4.9L19 9.7l-5.2 1.8L12 16.4l-1.8-4.9L5 9.7l5.2-1.8z" />
        <path d="M18.5 15l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7z" />
      {:else if kind === "command"}
        <rect x="3.5" y="4.5" width="17" height="15" rx="3" />
        <path d="M7.5 10l3 2.5-3 2.5M12.5 15.5h4" />
      {:else if kind === "folder"}
        <path d="M3.5 7.5a2 2 0 012-2h4l2 2h7a2 2 0 012 2v8a2 2 0 01-2 2h-13a2 2 0 01-2-2z" />
      {:else if kind === "file"}
        <path d="M6.5 3.5h7l4 4v13h-11z M13.5 3.5v4h4" />
      {:else}
        <rect x="4" y="4" width="7" height="7" rx="2" />
        <rect x="13" y="4" width="7" height="7" rx="2" />
        <rect x="4" y="13" width="7" height="7" rx="2" />
        <rect x="13" y="13" width="7" height="7" rx="2" />
      {/if}
    </svg>
  {/if}
</span>

<style>
  .icon {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: var(--icon-size);
    height: var(--icon-size);
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  svg {
    width: 80%;
    height: 80%;
    fill: none;
    stroke: var(--text-muted);
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  svg.ai {
    stroke: var(--accent);
  }
</style>
