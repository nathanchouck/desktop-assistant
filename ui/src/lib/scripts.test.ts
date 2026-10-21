import { describe, expect, it } from "vitest";
import type { ScriptInfo, ScriptsOverview } from "./bindings";
import { TIMEOUT_MAX, TIMEOUT_MIN, scriptDetail, scriptUsage, scriptsSummary, stepTimeout } from "./scripts";

const hello: ScriptInfo = {
  file: "hello.ps1",
  title: "Say Hello",
  description: "",
  icon: "text:👋",
  mode: "output",
  argument: "name",
  language: "PowerShell",
  in_explorer_folder: false,
};

const overview = (extra: Partial<ScriptsOverview> = {}): ScriptsOverview => ({
  folder: "C:\\scripts",
  default_folder: "C:\\scripts",
  folder_error: null,
  scripts: [],
  problems: [],
  ...extra,
});

describe("script commands in settings", () => {
  it("describes a script", () => {
    expect(scriptDetail(hello)).toBe("hello.ps1 · PowerShell · shows its output");
    expect(scriptDetail({ ...hello, mode: "silent", in_explorer_folder: true })).toBe(
      "hello.ps1 · PowerShell · runs quietly · in File Explorer's folder",
    );
  });

  it("says what to type", () => {
    expect(scriptUsage(hello)).toBe("say hello ‹name›");
    expect(scriptUsage({ ...hello, argument: null })).toBe("say hello");
  });

  it("sums up the folder", () => {
    expect(scriptsSummary(overview())).toBe("No script commands yet.");
    expect(scriptsSummary(overview({ scripts: [hello] }))).toBe("1 script command.");
    const problem = { file: "x.py", message: "No @title" };
    expect(scriptsSummary(overview({ scripts: [hello, hello], problems: [problem] }))).toBe(
      "2 script commands. 1 file isn't one (below).",
    );
    expect(scriptsSummary(overview({ problems: [problem, problem] }))).toBe(
      "No script commands yet. 2 files aren't (below).",
    );
    expect(scriptsSummary(overview({ folder_error: "The folder doesn't exist." }))).toBe("The folder doesn't exist.");
  });

  it("steps the time limit by 5 s up to a minute, then by 30 s", () => {
    expect(stepTimeout(60, true)).toBe(90);
    expect(stepTimeout(90, false)).toBe(60);
    expect(stepTimeout(60, false)).toBe(55);
    expect(stepTimeout(5, false)).toBe(TIMEOUT_MIN);
    expect(stepTimeout(600, true)).toBe(TIMEOUT_MAX);
    expect(stepTimeout(570, true)).toBe(600);
    // A hand-edited value goes to the nearest step that way.
    expect(stepTimeout(47, true)).toBe(50);
    expect(stepTimeout(65, false)).toBe(60);
    expect(stepTimeout(2000, false)).toBe(600);
  });
});
