import type { ScriptInfo, ScriptMode, ScriptsOverview } from "./bindings";

/** What Enter does with a script's output, as the settings list says it. */
const MODE_TEXT: Record<ScriptMode, string> = {
  silent: "runs quietly",
  output: "shows its output",
  copy: "copies its output",
  paste: "pastes its output",
};

/** "hello.ps1 · PowerShell · shows its output · in File Explorer's folder" */
export function scriptDetail(script: ScriptInfo): string {
  const parts = [script.file, script.language, MODE_TEXT[script.mode]];
  if (script.in_explorer_folder) parts.push("in File Explorer's folder");
  return parts.join(" · ");
}

/** What to type to run it: "say hello ‹name›". */
export function scriptUsage(script: ScriptInfo): string {
  const title = script.title.toLowerCase();
  return script.argument ? `${title} ‹${script.argument}›` : title;
}

/** One line on what the folder holds. */
export function scriptsSummary(overview: ScriptsOverview): string {
  if (overview.folder_error) return overview.folder_error;
  const found = overview.scripts.length;
  const commands =
    found === 0 ? "No script commands yet." : found === 1 ? "1 script command." : `${found} script commands.`;
  const other = overview.problems.length;
  if (other === 0) return commands;
  return `${commands} ${other === 1 ? "1 file isn't one" : `${other} files aren't`} (below).`;
}

/**
 * The time limits the settings offer, in seconds: 5 s steps up to a
 * minute, then 30 s steps up to 10 minutes, so both a quick 10 s and a
 * 5-minute backup are a few clicks away.
 */
const TIMEOUTS: number[] = [
  ...Array.from({ length: 12 }, (_, i) => (i + 1) * 5),
  ...Array.from({ length: 18 }, (_, i) => 90 + i * 30),
];
export const TIMEOUT_MIN = 5;
export const TIMEOUT_MAX = 600;

/** The next time limit up or down from `current`, which may be hand-edited and off the steps. */
export function stepTimeout(current: number, up: boolean): number {
  if (up) return TIMEOUTS.find((t) => t > current) ?? TIMEOUT_MAX;
  return [...TIMEOUTS].reverse().find((t) => t < current) ?? TIMEOUT_MIN;
}
