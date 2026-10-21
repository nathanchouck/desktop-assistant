<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    children: Snippet;
  }
  let { title, children }: Props = $props();
</script>

<section>
  <h3>{title}</h3>
  {@render children()}
</section>

<style>
  h3 {
    margin: var(--space-4) 0 var(--space-1);
    font-size: var(--font-size-sub);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
</style>
