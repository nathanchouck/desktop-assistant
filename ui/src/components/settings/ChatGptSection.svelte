<script lang="ts">
  // ChatGPT through OpenAI's Codex CLI: status, the default AI, model,
  // approvals, screen context, billing.
  import { commands, type BillingMode, type CodexStatus, type ProviderId } from "../../lib/bindings";
  import type { SettingsStore } from "../../lib/stores/settings.svelte";
  import Button from "./Button.svelte";
  import Choice from "./Choice.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Status from "./Status.svelte";
  import TextField from "./TextField.svelte";
  import Toggle from "./Toggle.svelte";

  let { store }: { store: SettingsStore } = $props();

  /** "" leaves the choice to Codex (its own config, else its default). */
  const MODELS: [string, string][] = [
    ["", "Default"],
    ["gpt-6.1-sol", "GPT-6.1-Sol"],
    ["gpt-6-luna", "GPT-6-Luna"],
  ];
  const CODEX_STATUS: Record<CodexStatus, string> = {
    ready: "Codex is installed and signed in to ChatGPT.",
    not_installed: "Codex isn't installed.",
    signed_out: "Codex isn't signed in to ChatGPT.",
    unknown: "Checking Codex…",
  };
</script>

{#if store.settings}
  {@const s = store.settings}
  {@const status = store.status}
  <Section title="ChatGPT">
    {#if status}
      <Status>
        <span>{CODEX_STATUS[status.chatgpt]}</span>
        {#if status.chatgpt === "not_installed"}
          <Button onclick={() => void commands.itemActivate("cmd:codex-install", "run")}>Install</Button>
        {:else if status.chatgpt === "signed_out"}
          <Button onclick={() => void commands.itemActivate("cmd:chatgpt-sign-in", "run")}>Sign in</Button>
        {/if}
      </Status>
    {/if}
    <Row label="Default AI" hint="Tab and the Ask row use it; Shift+Tab asks the other. Start with “c ” or “g ” to pick one.">
      <Choice
        options={[
          ["claude", "Claude"],
          ["chatgpt", "ChatGPT"],
        ]}
        value={s.ai.default_provider}
        label="Default AI"
        onchange={(v) => void store.save((n) => (n.ai.default_provider = v as ProviderId))}
      />
    </Row>
    <Row label="Model" hint="Default follows Codex's own setting. Luna is faster and uses less of your plan.">
      <Choice
        options={MODELS.some(([key]) => key === s.ai.chatgpt.model)
          ? MODELS
          : [...MODELS, [s.ai.chatgpt.model, s.ai.chatgpt.model]]}
        value={s.ai.chatgpt.model}
        label="Model"
        onchange={(model) => void store.save((n) => (n.ai.chatgpt.model = model))}
      />
    </Row>
    <Row
      label="Approvals"
      hint={s.ai.chatgpt.approvals === "ask"
        ? "ChatGPT can look around; you approve anything that changes files or goes online."
        : "Changes files in the Explorer folder you came from without asking (offline); asks for anything else. Screenshots always ask."}
    >
      <Choice
        options={[
          ["ask", "Ask me"],
          ["auto", "Auto"],
        ]}
        value={s.ai.chatgpt.approvals}
        label="Approvals"
        onchange={(v) => void store.save((n) => (n.ai.chatgpt.approvals = v as "ask" | "auto"))}
      />
    </Row>
    <Row label="Screen context" hint="Tell ChatGPT which window you came from, and Explorer's folder and selection.">
      <Toggle
        on={s.ai.chatgpt.desktop_context}
        label="Screen context"
        onchange={(on) => void store.save((n) => (n.ai.chatgpt.desktop_context = on))}
      />
    </Row>
    <Row label="Start Codex early" hint="While you type a question for ChatGPT, so the answer starts sooner.">
      <Toggle
        on={s.ai.chatgpt.warm_start}
        label="Start Codex early"
        onchange={(on) => void store.save((n) => (n.ai.chatgpt.warm_start = on))}
      />
    </Row>
    <Row
      label="Billing"
      hint={s.ai.chatgpt.billing === "subscription"
        ? "Uses your ChatGPT plan through Codex's sign-in."
        : "Uses the OpenAI API key saved under API keys (else OPENAI_API_KEY), billed per token."}
    >
      <Choice
        options={[
          ["subscription", "My plan"],
          ["api_key", "API key"],
        ]}
        value={s.ai.chatgpt.billing}
        label="Billing"
        onchange={(v) => void store.save((n) => (n.ai.chatgpt.billing = v as BillingMode))}
      />
    </Row>
    <Row label="Codex location" hint="Leave empty to find it (PATH, the installer's folder, npm, WinGet).">
      <TextField
        value={s.ai.chatgpt.path}
        placeholder="Found automatically"
        label="Codex location"
        mono
        onchange={(path) => void store.save((n) => (n.ai.chatgpt.path = path.trim()))}
      />
    </Row>
  </Section>
{/if}
