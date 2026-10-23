import { describe as group, expect, it } from "vitest";
import type { McpServer, SecretStatus } from "./bindings";
import {
  API_KEYS,
  byName,
  describe,
  draftFrom,
  editFrom,
  missingSecrets,
  newDraft,
  savedLabel,
  secretName,
  secretNames,
  valuePlaceholder,
} from "./mcp";

const github: McpServer = {
  name: "github",
  enabled: true,
  transport: "stdio",
  command: "npx",
  args: ["-y", "@modelcontextprotocol/server-github"],
  env: [
    { name: "GITHUB_TOKEN", value: "", secret: true },
    { name: "LOG", value: "info", secret: false },
  ],
  url: "",
  headers: [],
};

const linear: McpServer = {
  name: "linear",
  enabled: false,
  transport: "http",
  command: "",
  args: [],
  env: [],
  url: "https://mcp.linear.app/mcp",
  headers: [{ name: "Authorization", value: "", secret: true }],
};

const saved = (name: string, last4: string | null = "1234"): SecretStatus => ({ name, saved: true, last4 });

group("secret names", () => {
  it("match the Rust side's format", () => {
    expect(secretName("github", "env", "GITHUB_TOKEN")).toBe("mcp:github:env:GITHUB_TOKEN");
    expect(secretNames(linear)).toEqual(["mcp:linear:header:Authorization"]);
    expect(secretNames(github)).toEqual(["mcp:github:env:GITHUB_TOKEN"]);
  });

  it("lists API keys by their Credential Manager names", () => {
    expect(API_KEYS.map((k) => k.name)).toEqual(["anthropic-api-key", "openai-api-key"]);
  });
});

group("missingSecrets", () => {
  it("names secrets not saved on this PC", () => {
    expect(missingSecrets(github, new Map())).toEqual(["GITHUB_TOKEN"]);
    const statuses = byName([saved("mcp:github:env:GITHUB_TOKEN")]);
    expect(missingSecrets(github, statuses)).toEqual([]);
    const notSaved = byName([{ name: "mcp:github:env:GITHUB_TOKEN", saved: false, last4: null }]);
    expect(missingSecrets(github, notSaved)).toEqual(["GITHUB_TOKEN"]);
  });
});

group("describe", () => {
  it("shows the command line or the address", () => {
    expect(describe(github)).toBe("npx -y @modelcontextprotocol/server-github");
    expect(describe(linear)).toBe("https://mcp.linear.app/mcp");
    expect(describe({ ...github, command: "C:\\My Tools\\srv.exe", args: ["--dir", "C:\\a b"] })).toBe(
      '"C:\\My Tools\\srv.exe" --dir "C:\\a b"',
    );
  });
});

group("savedLabel", () => {
  it("shows only whether it's saved and the last four", () => {
    expect(savedLabel(saved("x"))).toBe("Saved · ends in 1234");
    expect(savedLabel(saved("x", null))).toBe("Saved");
    expect(savedLabel(undefined)).toBe("Not saved");
    expect(savedLabel({ name: "x", saved: false, last4: null })).toBe("Not saved");
  });
});

group("drafts", () => {
  it("round-trip a server", () => {
    expect(editFrom(draftFrom(github))).toEqual({ previous: "github", server: github });
    expect(editFrom(draftFrom(linear))).toEqual({ previous: "linear", server: linear });
  });

  it("start new servers empty, as stdio", () => {
    const d = newDraft();
    expect(d.previous).toBeNull();
    expect(d.transport).toBe("stdio");
    expect(editFrom(d).server.args).toEqual([]);
  });

  it("take arguments one per line and drop blank rows", () => {
    const d = draftFrom(github);
    d.name = "  gh ";
    d.args = "-y\r\n  @scope/pkg  \n\n--flag value with spaces\n";
    d.env.push({ name: " ", value: "  ", secret: false, original: null });
    d.env.push({ name: "NEW", value: "", secret: false, original: null });
    const { previous, server } = editFrom(d);
    expect(previous).toBe("github");
    expect(server.name).toBe("gh");
    expect(server.args).toEqual(["-y", "@scope/pkg", "--flag value with spaces"]);
    expect(server.env.map((v) => v.name)).toEqual(["GITHUB_TOKEN", "LOG", "NEW"]);
  });

  it("never prefill a secret's value", () => {
    const withValue = { ...github, env: [{ name: "GITHUB_TOKEN", value: "leak", secret: true }] };
    expect(draftFrom(withValue).env[0]!.value).toBe("");
  });
});

group("valuePlaceholder", () => {
  const statuses = byName([saved("mcp:github:env:GITHUB_TOKEN", "9xyz")]);

  it("says a saved secret is kept when left empty", () => {
    const d = draftFrom(github);
    expect(valuePlaceholder(d, "env", d.env[0]!, statuses)).toBe("Saved · ends in 9xyz · type to replace");
    // Renaming the server keeps it too.
    d.name = "gh";
    expect(valuePlaceholder(d, "env", d.env[0]!, statuses)).toContain("ends in 9xyz");
  });

  it("asks for a value when there's nothing saved under that name", () => {
    const d = draftFrom(github);
    d.env[0]!.name = "GH_TOKEN";
    expect(valuePlaceholder(d, "env", d.env[0]!, statuses)).toBe("Secret value (kept in Credential Manager)");
    expect(valuePlaceholder(d, "env", d.env[0]!, new Map())).toContain("Credential Manager");
    expect(valuePlaceholder(d, "env", d.env[1]!, statuses)).toBe("Value");
    const fresh = newDraft();
    const row = { name: "TOKEN", value: "", secret: true, original: null };
    expect(valuePlaceholder(fresh, "env", row, statuses)).toContain("Credential Manager");
  });
});
