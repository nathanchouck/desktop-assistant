<script lang="ts">
  // Conversation history: on/off, how many chats to keep and for how long,
  // how many are saved, and Clear.
  import { commands, type HistoryOverview } from "../../lib/bindings";
  import { clearable, forgetOptions, historySummary, KEEP_MAX, KEEP_MIN, KEEP_STEP } from "../../lib/history";
  import { CONFIRM_MS } from "../../lib/stores/search.svelte";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Button from "./Button.svelte";
  import Choice from "./Choice.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Status from "./Status.svelte";
  import Stepper from "./Stepper.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();
  let overview = $state<HistoryOverview | null>(null);
  /** Clear was pressed once; pressing it again within a few seconds clears. */
  let confirming = $state(false);
  let confirmTimer: ReturnType<typeof setTimeout> | undefined;

  /** How many chats are saved (after applying the limits). */
  async function load() {
    const res = await commands.historyOverview();
    if (res.status === "ok") overview = res.data;
    else store.error = res.error.message;
  }

  $effect(() => {
    void load();
    return () => clearTimeout(confirmTimer);
  });

  async function save(change: Parameters<SettingsStore["save"]>[0]) {
    // A lower limit forgets chats at once; show the new count.
    if (await store.save(change)) await load();
  }

  async function clear() {
    if (!confirming) {
      confirming = true;
      confirmTimer = setTimeout(() => (confirming = false), CONFIRM_MS);
      return;
    }
    clearTimeout(confirmTimer);
    confirming = false;
    const res = await commands.historyClear();
    if (res.status === "ok") overview = res.data;
    else store.error = res.error.message;
  }
</script>

{#if store.settings}
  {@const h = store.settings.history}
  <Section title="Chat history">
    <Row
      label="Keep chat history"
      hint="Type history in the launcher to find a past chat; Enter carries it on. Off: new chats aren't saved, and saved ones stay until you clear them."
    >
      <Toggle on={h.enabled} label="Keep chat history" onchange={(on) => void save((s) => (s.history.enabled = on))} />
    </Row>
    {#if h.enabled}
      <Row label="Chats kept" hint="The oldest go first. Pinned chats (Ctrl+Shift+P) don't count.">
        <Stepper
          value={h.max_conversations}
          min={KEEP_MIN}
          max={KEEP_MAX}
          step={KEEP_STEP}
          onchange={(v) => void save((s) => (s.history.max_conversations = v))}
        />
      </Row>
      <Row label="Forget chats after" hint="Counted from when you last used a chat. Pinned chats stay.">
        <Choice
          options={forgetOptions(h.forget_after_days)}
          value={String(h.forget_after_days)}
          label="Forget chats after"
          onchange={(v) => void save((s) => (s.history.forget_after_days = Number(v)))}
        />
      </Row>
    {/if}
    <Status>
      <span>{overview ? historySummary(overview) : "Counting saved chats…"}</span>
      <Button disabled={!overview || clearable(overview) === 0} onclick={() => void clear()}>
        {confirming ? "Press again to clear" : "Clear"}
      </Button>
    </Status>
  </Section>
{/if}
