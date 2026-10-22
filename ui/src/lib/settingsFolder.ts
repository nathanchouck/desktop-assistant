// Wording and choices for Settings → Advanced → Settings folder.

import type { FolderCheck, FolderMoved, SettingsFolder } from "./bindings";

/** A button for moving to a folder. */
export interface MoveChoice {
  label: string;
  /** Write this PC's settings there even if it has some. */
  replace: boolean;
  primary: boolean;
}

/** A folder with settings in it (another PC's) can be used as it is, or
 * replaced with this PC's; an empty one just gets this PC's. */
export function moveChoices(target: FolderCheck): MoveChoice[] {
  if (!target.has_settings) return [{ label: "Move settings here", replace: false, primary: true }];
  return [
    { label: "Use the settings there", replace: false, primary: true },
    { label: "Replace with this PC's", replace: true, primary: false },
  ];
}

/** The line under the folder: where settings are and what that means. */
export function folderHint(folder: SettingsFolder): string {
  if (folder.is_default) return "Only this PC uses them. Pick a synced folder (OneDrive) to share them with your other PCs.";
  return "Shared with every PC that uses this folder. Changes made on another PC apply here once they sync.";
}

/** What happened after a move, for the notice under the section. */
export function movedMessage(moved: FolderMoved, before: SettingsFolder): string {
  const to = moved.location.folder;
  if (moved.adopted) return `Now using the settings in ${to}. This PC's own settings stayed as they were.`;
  if (moved.location.is_default) {
    const left = before.is_default ? "" : ` The copy in ${before.folder} is left for your other PCs.`;
    return `Settings are back in ${to}.${left}`;
  }
  const backup = moved.backup ? ` The file that was there is kept as ${fileName(moved.backup)}.` : "";
  return `Settings are now in ${to}.${backup}`;
}

function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}
