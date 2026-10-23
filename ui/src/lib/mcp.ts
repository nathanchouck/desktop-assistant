// Settings → Integrations: API keys and the MCP registry. Pure helpers for
// the section and its server editor; the backend checks and saves.
import type { McpServer, McpServerEdit, McpTransport, McpValue, SecretStatus } from "./bindings";

/** An API key the launcher keeps in Credential Manager. */
export interface ApiKeyInfo {
  /** Its name in Credential Manager (after "desktop-assistant:"). */
  name: string;
  label: string;
  hint: string;
  placeholder: string;
}

export const API_KEYS: ApiKeyInfo[] = [
  {
    name: "anthropic-api-key",
    label: "Anthropic API key",
    hint: "Used when Claude's billing is API key (else ANTHROPIC_API_KEY).",
    placeholder: "sk-ant-…",
  },
  {
    name: "openai-api-key",
    label: "OpenAI API key",
    hint: "Used when ChatGPT's billing is API key (else OPENAI_API_KEY).",
    placeholder: "sk-…",
  },
];

/** Which list a value is in: a stdio server's environment, or an HTTP server's headers. */
export type ValueKind = "env" | "header";

export function kindOf(transport: McpTransport): ValueKind {
  return transport === "stdio" ? "env" : "header";
}

/** A secret value's name in Credential Manager. Must match `mcp_edit::secret_name` in Rust. */
export function secretName(server: string, kind: ValueKind, entry: string): string {
  return `mcp:${server}:${kind}:${entry}`;
}

/** The values a server's transport uses. */
function valuesOf(server: McpServer): McpValue[] {
  return server.transport === "stdio" ? server.env : server.headers;
}

/** Credential Manager names of a server's secret values. */
export function secretNames(server: McpServer): string[] {
  const kind = kindOf(server.transport);
  return valuesOf(server)
    .filter((v) => v.secret)
    .map((v) => secretName(server.name, kind, v.name));
}

/** Secret values of `server` that aren't saved on this PC (a server synced from another PC). */
export function missingSecrets(server: McpServer, statuses: Map<string, SecretStatus>): string[] {
  const kind = kindOf(server.transport);
  return valuesOf(server)
    .filter((v) => v.secret && statuses.get(secretName(server.name, kind, v.name))?.saved !== true)
    .map((v) => v.name);
}

/** What one row of the server list says under its name. */
export function describe(server: McpServer): string {
  if (server.transport === "http") return server.url;
  const quote = (part: string) => (/\s/.test(part) ? `"${part}"` : part);
  return [server.command, ...server.args].map(quote).join(" ");
}

/** "Saved · ends in 1234" for a saved secret, else "Not saved". */
export function savedLabel(status: SecretStatus | undefined): string {
  if (!status?.saved) return "Not saved";
  return status.last4 ? `Saved · ends in ${status.last4}` : "Saved";
}

/** One environment variable or header in the editor. */
export interface DraftValue {
  name: string;
  /** For a secret: the new value, or empty to keep the saved one. */
  value: string;
  secret: boolean;
  /** Its name when the editor opened (`null` for a new row), to find its saved secret. */
  original: string | null;
}

/** The editor's working copy of a server. Arguments are one per line. */
export interface ServerDraft {
  /** The server's name when the editor opened; `null` for a new server. */
  previous: string | null;
  name: string;
  enabled: boolean;
  transport: McpTransport;
  command: string;
  args: string;
  url: string;
  env: DraftValue[];
  headers: DraftValue[];
}

export function newDraft(): ServerDraft {
  return {
    previous: null,
    name: "",
    enabled: true,
    transport: "stdio",
    command: "",
    args: "",
    url: "",
    env: [],
    headers: [],
  };
}

const toDraftValues = (values: McpValue[]): DraftValue[] =>
  values.map((v) => ({ name: v.name, value: v.secret ? "" : v.value, secret: v.secret, original: v.name }));

export function draftFrom(server: McpServer): ServerDraft {
  return {
    previous: server.name,
    name: server.name,
    enabled: server.enabled,
    transport: server.transport,
    command: server.command,
    args: server.args.join("\n"),
    url: server.url,
    env: toDraftValues(server.env),
    headers: toDraftValues(server.headers),
  };
}

/** Rows left completely blank are dropped. */
const fromDraftValues = (values: DraftValue[]): McpValue[] =>
  values
    .map((v) => ({ name: v.name.trim(), value: v.value, secret: v.secret }))
    .filter((v) => v.name !== "" || v.value.trim() !== "");

/** What the editor sends to `mcp_server_save`. */
export function editFrom(draft: ServerDraft): McpServerEdit {
  return {
    previous: draft.previous,
    server: {
      name: draft.name.trim(),
      enabled: draft.enabled,
      transport: draft.transport,
      command: draft.command.trim(),
      args: draft.args
        .split(/\r?\n/)
        .map((a) => a.trim())
        .filter((a) => a !== ""),
      url: draft.url.trim(),
      env: fromDraftValues(draft.env),
      headers: fromDraftValues(draft.headers),
    },
  };
}

/** The value field's placeholder: whether a saved secret is kept if it's left empty. */
export function valuePlaceholder(
  draft: ServerDraft,
  kind: ValueKind,
  value: DraftValue,
  statuses: Map<string, SecretStatus>,
): string {
  if (!value.secret) return "Value";
  const kept =
    draft.previous !== null &&
    value.original !== null &&
    value.original === value.name.trim() &&
    statuses.get(secretName(draft.previous, kind, value.original))?.saved === true;
  if (!kept) return "Secret value (kept in Credential Manager)";
  const status = statuses.get(secretName(draft.previous ?? "", kind, value.original ?? ""));
  return `${savedLabel(status)} · type to replace`;
}

/** Statuses by name, for lookups. */
export function byName(statuses: SecretStatus[]): Map<string, SecretStatus> {
  return new Map(statuses.map((s) => [s.name, s]));
}
