<script lang="ts">
  interface Props {
    on: boolean;
    /** For screen readers (the row's label). */
    label: string;
    onchange: (on: boolean) => void;
  }
  let { on, label, onchange }: Props = $props();
</script>

<button type="button" role="switch" aria-checked={on} aria-label={label} class:on onclick={() => onchange(!on)}>
  <span class="knob"></span>
</button>

<style>
  button {
    position: relative;
    flex: none;
    width: 36px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--keycap-border);
    border-radius: 10px;
    background: var(--switch-off);
    transition: background var(--duration-fast) var(--ease);
  }
  button.on {
    border-color: transparent;
    background: var(--accent);
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--switch-knob);
    transition: transform var(--duration-fast) var(--ease);
  }
  button.on .knob {
    transform: translateX(16px);
  }
</style>
