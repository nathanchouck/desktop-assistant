import { describe, expect, it } from "vitest";
import type { ResultItem } from "./bindings";
import { ACTION_LABELS, confirmQuestion, filterEntries, menuEntries } from "./actions";

const file: ResultItem = {
  id: "file:C:\\x\\report.pdf",
  kind: "file",
  title: "report.pdf",
  subtitle: "",
  icon: null,
  actions: ["open", "reveal_in_explorer", "copy_path", "open_with", "trash"],
  accessory: null,
  confirm: null,
};

describe("menuEntries", () => {
  it("lists every action of the row, the default first, with its key", () => {
    expect(menuEntries(file)).toEqual([
      { action: "open", label: "Open", key: "↵" },
      { action: "reveal_in_explorer", label: "Show in Folder", key: "Ctrl ↵" },
      { action: "copy_path", label: "Copy Path", key: "Ctrl ⇧ C" },
      { action: "open_with", label: "Open With…", key: null },
      { action: "trash", label: "Delete to Recycle Bin", key: "Ctrl D" },
    ]);
  });

  it("has a label for every action", () => {
    for (const label of Object.values(ACTION_LABELS)) expect(label).not.toBe("");
  });
});

describe("filterEntries", () => {
  const entries = menuEntries(file);
  const labels = (text: string) => filterEntries(entries, text).map((e) => e.label);

  it("keeps everything for an empty filter", () => {
    expect(labels("")).toHaveLength(5);
    expect(labels("   ")).toHaveLength(5);
  });

  it("matches the start of any word, ignoring case", () => {
    expect(labels("rec")).toEqual(["Delete to Recycle Bin"]);
    expect(labels("OPEN")).toEqual(["Open", "Open With…"]);
    expect(labels("op w")).toEqual(["Open With…"]);
    expect(labels("path")).toEqual(["Copy Path"]);
  });

  it("finds nothing for letters inside words", () => {
    expect(labels("ycle")).toEqual([]);
    expect(labels("xyz")).toEqual([]);
  });
});

describe("confirmQuestion", () => {
  it("always asks before the Recycle Bin", () => {
    expect(confirmQuestion(file, "trash")).toBe("Move report.pdf to the Recycle Bin?");
    expect(confirmQuestion(file, "open")).toBeNull();
    expect(confirmQuestion(file, "open_with")).toBeNull();
  });

  it("asks a row's own question for its default action only", () => {
    const restart = { ...file, id: "sys:restart", actions: ["run", "pin"], confirm: "Restart?" } as ResultItem;
    expect(confirmQuestion(restart, "run")).toBe("Restart?");
    expect(confirmQuestion(restart, "pin")).toBeNull();
  });
});
