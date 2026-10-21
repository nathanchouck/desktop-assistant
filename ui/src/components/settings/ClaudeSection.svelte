<script lang="ts">
  import { commands, type ClaudeStatus } from "../../lib/bindings";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Button from "./Button.svelte";
  import Choice from "./Choice.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Status from "./Status.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();

  const MODELS: [string, string][] = [
    ["sonnet", "Sonnet 5.5"],
    ["haiku", "Haiku 5.5"],
  ];
  const CLAUDE_STATUS: Record<ClaudeStatus, string> = {
    ready: "Claude Code is installed and signed in.",
    not_installed: "Claude Code isn't installed.",
    signed_out: "Claude Code isn't signed in.",
    unknown: "Checking Claude Code…",
  };
</script>

{#if store.settings}
  {@const s = store.settings}
  {@const status = store.status}
  <Section title="Claude">
    {#if status}
      <Status>
        <span>{CLAUDE_STATUS[status.claude]}</span>
        {#if status.claude === "not_installed"}
          <Button onclick={() => void commands.itemActivate("cmd:claude-install", "run")}>Install</Button>
        {:else if status.claude === "signed_out"}
          <Button onclick={() => void commands.itemActivate("cmd:claude-sign-in", "run")}>Sign in</Button>
        {/if}
      </Status>
    {/if}
    <Row label="Model" hint="Sonnet is smarter; Haiku is faster and uses less of your plan.">
      <Choice
        options={MODELS.some(([key]) => key === s.ai.claude.model) ? MODELS : [...MODELS, [s.ai.claude.model, s.ai.claude.model]]}
        value={s.ai.claude.model}
        label="Model"
        onchange={(model) => void store.save((n) => (n.ai.claude.model = model))}
      />
    </Row>
    <Row
      label="Approvals"
      hint={s.ai.claude.approvals === "ask"
        ? "You approve anything that changes files or runs commands."
        : "Claude Code decides routine actions; it asks when unsure. Screenshots always ask."}
    >
      <Choice
        options={[
          ["ask", "Ask me"],
          ["auto", "Auto"],
        ]}
        value={s.ai.claude.approvals}
        label="Approvals"
        onchange={(v) => void store.save((n) => (n.ai.claude.approvals = v as "ask" | "auto"))}
      />
    </Row>
    <Row label="Screen context" hint="Tell Claude which window you came from, and Explorer's folder and selection.">
      <Toggle
        on={s.ai.claude.desktop_context}
        label="Screen context"
        onchange={(on) => void store.save((n) => (n.ai.claude.desktop_context = on))}
      />
    </Row>
    <Row label="Start Claude early" hint="While you type a question, so the answer starts about a second sooner.">
      <Toggle
        on={s.ai.claude.warm_start}
        label="Start Claude early"
        onchange={(on) => void store.save((n) => (n.ai.claude.warm_start = on))}
      />
    </Row>
    <Row
      label="Billing"
      hint={s.ai.claude.billing === "subscription"
        ? "Uses your Claude plan through Claude Code's sign-in."
        : "Uses the ANTHROPIC_API_KEY environment variable."}
    >
      <Choice
        options={[
          ["subscription", "My plan"],
          ["api_key", "API key"],
        ]}
        value={s.ai.claude.billing}
        label="Billing"
        onchange={(v) => void store.save((n) => (n.ai.claude.billing = v as "subscription" | "api_key"))}
      />
    </Row>
  </Section>
{/if}
