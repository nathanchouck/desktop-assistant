<script lang="ts">
  interface Props {
    value: number;
    min: number;
    max: number;
    step?: number;
    onchange: (value: number) => void;
  }
  let { value, min, max, step = 1, onchange }: Props = $props();
</script>

<div>
  <button type="button" aria-label="Fewer" disabled={value <= min} onclick={() => onchange(Math.max(min, value - step))}>−</button>
  <span>{value}</span>
  <button type="button" aria-label="More" disabled={value >= max} onclick={() => onchange(Math.min(max, value + step))}>+</button>
</div>

<style>
  div {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  span {
    min-width: 2ch;
    text-align: center;
  }
  button {
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--keycap-border);
    border-radius: var(--radius-keycap);
    background: var(--keycap);
    color: var(--text);
    font: inherit;
    font-size: var(--font-size-sub);
  }
  button:disabled {
    opacity: 0.4;
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
