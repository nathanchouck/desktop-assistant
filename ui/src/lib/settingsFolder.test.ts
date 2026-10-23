import { describe, expect, it } from "vitest";
import type { SettingsFolder } from "./bindings";
import { folderHint, missingNotice, moveChoices, movedMessage } from "./settingsFolder";

const DEFAULT = String.raw`C:\Users\me\AppData\Roaming\desktop-assistant`;
const ONEDRIVE = String.raw`C:\Users\me\OneDrive\desktop-assistant`;

function folder(over: Partial<SettingsFolder> = {}): SettingsFolder {
  return {
    folder: DEFAULT,
    is_default: true,
    default_folder: DEFAULT,
    local_file: String.raw`C:\Users\me\AppData\Local\desktop-assistant\settings.local.toml`,
    missing: null,
    unreadable: null,
    suggestion: null,
    ...over,
  };
}

describe("moveChoices", () => {
  it("offers one move to an empty folder", () => {
    expect(moveChoices({ folder: ONEDRIVE, has_settings: false })).toEqual([
      { label: "Move settings here", replace: false, primary: true },
    ]);
  });

  it("offers to use or replace another PC's settings", () => {
    const choices = moveChoices({ folder: ONEDRIVE, has_settings: true });
    expect(choices.map((c) => [c.label, c.replace])).toEqual([
      ["Use the settings there", false],
      ["Replace with this PC's", true],
    ]);
  });
});

describe("folderHint", () => {
  it("says whether settings are shared", () => {
    expect(folderHint(folder())).toMatch(/^Only this PC/);
    expect(folderHint(folder({ folder: ONEDRIVE, is_default: false }))).toMatch(/^Shared/);
  });
});

describe("missingNotice", () => {
  it("says what's used while a chosen folder is gone", () => {
    expect(missingNotice(folder())).toBeNull();
    expect(missingNotice(folder({ missing: ONEDRIVE }))).toBe(
      `${ONEDRIVE} isn't there right now, so settings come from the default folder until it is.`,
    );
    expect(missingNotice(folder({ folder: ONEDRIVE, is_default: false, missing: ONEDRIVE }))).toMatch(
      /isn't there right now, so the settings in use stay as they are/,
    );
  });
});

describe("movedMessage", () => {
  const synced = folder({ folder: ONEDRIVE, is_default: false });

  it("describes a move with and without a backup", () => {
    expect(movedMessage({ location: synced, adopted: false, backup: null }, folder())).toBe(
      `Settings are now in ${ONEDRIVE}.`,
    );
    expect(
      movedMessage({ location: synced, adopted: false, backup: `${ONEDRIVE}\\settings.toml.bak` }, folder()),
    ).toBe(`Settings are now in ${ONEDRIVE}. The file that was there is kept as settings.toml.bak.`);
  });

  it("describes using another PC's settings", () => {
    expect(movedMessage({ location: synced, adopted: true, backup: null }, folder())).toMatch(
      /^Now using the settings in .*OneDrive/,
    );
  });

  it("describes going back to the default folder", () => {
    const back = { location: folder(), adopted: false, backup: `${DEFAULT}\\settings.toml.bak` };
    expect(movedMessage(back, synced)).toBe(
      `Settings are back in ${DEFAULT}. The copy in ${ONEDRIVE} is left for your other PCs.`,
    );
    expect(movedMessage(back, folder())).toBe(`Settings are back in ${DEFAULT}.`);
  });
});
