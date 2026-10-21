<script lang="ts">
  interface Props {
    value: string;
    placeholder?: string;
    /** For screen readers (the row's label). */
    label: string;
    /** Monospace, for paths and keywords. */
    mono?: boolean;
    /** Many lines (snippet text, prompts). */
    multiline?: boolean;
    /** Called with the new text on Enter (single line) or when focus leaves. */
    onchange: (value: string) => void;
  }
  let { value, placeholder = "", label, mono = false, multiline = false, onchange }: Props = $props();
  let draft = $state("");

  // Follow the saved value until you start editing.
  $effect(() => {
    draft = value;
  });

  function commit() {
    if (draft !== value) onchange(draft);
  }
</script>

{#if multiline}
  <textarea bind:value={draft} {placeholder} aria-label={label} class:mono spellcheck="false" rows="3" onblur={commit}
  ></textarea>
{:else}
  <input
    bind:value={draft}
    {placeholder}
    aria-label={label}
    class:mono
    spellcheck="false"
    onblur={commit}
    onkeydown={(e) => {
      if (e.key === "Enter") commit();
    }}
  />
{/if}

<style>
  input,
  textarea {
    min-width: 0;
    padding: 2px var(--space-2);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--font-size-sub);
  }
  textarea {
    width: 100%;
    resize: vertical;
  }
  .mono {
    font-family: var(--font-mono);
  }
  input:focus-visible,
  textarea:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
