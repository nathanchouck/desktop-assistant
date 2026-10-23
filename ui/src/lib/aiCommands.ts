import type { AiCommand, CommandAi } from "./bindings";

/** Placeholders a saved prompt can use (see `features/ai_commands/template.rs`). */
export const PLACEHOLDERS = ["{selection}", "{clipboard}", "{text}", "{date}", "{time}", "{text|clipboard}"];

/** Who answers a saved command, for the settings menu. */
export const AI_CHOICES: [CommandAi, string][] = [
  ["default", "Default AI"],
  ["claude", "Claude"],
  ["chatgpt", "ChatGPT"],
];

/** A new command with a name no other command has. */
export function newCommand(existing: AiCommand[]): AiCommand {
  const taken = new Set(existing.map((c) => c.name.trim().toLowerCase()));
  let name = "New command";
  for (let n = 2; taken.has(name.toLowerCase()); n++) name = `New command ${n}`;
  return { name, keyword: "", prompt: "{text|clipboard}", provider: "default", icon: "" };
}

/**
 * Problems with each command, by index: no name or prompt, or a name or
 * keyword another command already has (search finds both, and Enter runs
 * the first one with that name).
 */
export function commandWarnings(commands: AiCommand[]): string[][] {
  // Enter finds a command by its exact name, so case counts there.
  const names = commands.map((c) => c.name.trim());
  const keywords = commands.map((c) => c.keyword.trim().toLowerCase());
  return commands.map((command, i) => {
    const warnings: string[] = [];
    if (!names[i]) warnings.push("Needs a name to show up in search.");
    else if (names.indexOf(names[i] ?? "") < i) warnings.push("Another command has this name, so this one never runs.");
    const keyword = keywords[i];
    if (keyword) {
      const twin = keywords.findIndex((k, j) => j !== i && k === keyword);
      if (twin >= 0) warnings.push(`Same keyword as “${commands[twin]?.name.trim() || "another command"}”.`);
    }
    if (!command.prompt.trim()) warnings.push("The prompt is empty.");
    return warnings;
  });
}
