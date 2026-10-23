import { describe, expect, it } from "vitest";
import type { AiCommand } from "./bindings";
import { commandWarnings, newCommand } from "./aiCommands";

const cmd = (name: string, keyword = "", prompt = "Fix: {text}"): AiCommand => ({
  name,
  keyword,
  prompt,
  provider: "default",
  icon: "",
});

describe("newCommand", () => {
  it("picks a name nobody has", () => {
    expect(newCommand([]).name).toBe("New command");
    expect(newCommand([cmd("New command"), cmd("new command 2")]).name).toBe("New command 3");
    expect(newCommand([]).provider).toBe("default");
  });
});

describe("commandWarnings", () => {
  it("is quiet for a good list", () => {
    expect(commandWarnings([cmd("Fix Grammar", "fix"), cmd("Translate")])).toEqual([[], []]);
  });

  it("flags missing names and prompts", () => {
    const [warnings] = commandWarnings([cmd("  ", "", " ")]);
    expect(warnings).toEqual(["Needs a name to show up in search.", "The prompt is empty."]);
  });

  it("flags repeated names and keywords", () => {
    const warnings = commandWarnings([cmd("Fix", "f"), cmd("Fix ", "F"), cmd("fix", "o")]);
    expect(warnings[0]).toEqual(["Same keyword as “Fix”."]);
    expect(warnings[1]).toEqual([
      "Another command has this name, so this one never runs.",
      "Same keyword as “Fix”.",
    ]);
    expect(warnings[2]).toEqual([]);
  });
});
