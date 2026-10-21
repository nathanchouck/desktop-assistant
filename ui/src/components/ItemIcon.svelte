<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import type { ItemKind } from "../lib/bindings";
  import { fallback, iconSpec } from "../lib/glyphs";

  interface Props {
    kind: ItemKind;
    /** See `ResultItem::icon`: an icon key, `glyph:`, `text:` or `color:`. */
    icon: string | null;
  }
  let { kind, icon }: Props = $props();
  let broken = $state(false);

  // A new icon key (row reused for another item) gets a fresh chance.
  $effect(() => {
    void icon;
    broken = false;
  });

  const spec = $derived(broken ? fallback(kind) : iconSpec(kind, icon));
</script>

<span class="icon" aria-hidden="true">
  {#if spec.type === "image"}
    <img src={convertFileSrc(spec.key, "icon")} alt="" draggable="false" onerror={() => (broken = true)} />
  {:else if spec.type === "text"}
    <span class="text">{spec.text}</span>
  {:else if spec.type === "color"}
    <span class="swatch" style:background={spec.color}></span>
  {:else}
    <!-- Static markup from glyphs.ts, never result text. -->
    <svg viewBox="0 0 24 24" class:ai={spec.accent}>{@html spec.markup}</svg>
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
    border-radius: var(--radius-icon);
  }
  .text {
    font-size: calc(var(--icon-size) * 0.75);
    line-height: 1;
  }
  .swatch {
    width: 80%;
    height: 80%;
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-icon);
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
