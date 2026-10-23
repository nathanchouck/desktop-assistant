import { describe, expect, it } from "vitest";
import {
  chipLabel,
  chipName,
  composedQuestion,
  DEFAULT_QUESTION,
  fileName,
  mergeAttachments,
  pictureKey,
  textPreview,
} from "./compose";

describe("chips", () => {
  it("label each kind of attachment briefly", () => {
    expect(chipLabel({ type: "text", text: "  teh   cat\nsat  " })).toBe("“teh cat sat”");
    expect(chipLabel({ type: "text", text: "a".repeat(40) })).toBe(`“${"a".repeat(27)}…”`);
    expect(chipLabel({ type: "path", path: "C:\\Users\\me\\report.pdf", folder: false })).toBe("report.pdf");
    expect(chipLabel({ type: "path", path: "C:\\Users\\me\\Downloads\\", folder: true })).toBe("Downloads");
    expect(chipLabel({ type: "image", id: "1a", width: 800, height: 600 })).toBe("Picture 800×600");
  });

  it("name what a remove button removes", () => {
    expect(chipName({ type: "text", text: "x" })).toBe("the selected text");
    expect(chipName({ type: "path", path: "/home/me/a.txt", folder: false })).toBe("a.txt");
    expect(chipName({ type: "image", id: "1a", width: 1, height: 1 })).toBe("the picture");
  });

  it("find pictures' previews", () => {
    expect(pictureKey("19af")).toBe("thumb/attach-19af");
  });
});

describe("fileName", () => {
  it("takes the last part of either kind of path", () => {
    expect(fileName("C:\\a\\b.txt")).toBe("b.txt");
    expect(fileName("/a/b/")).toBe("b");
    expect(fileName("C:\\")).toBe("C:");
  });
});

describe("mergeAttachments", () => {
  it("keeps the order and skips repeats", () => {
    const a = { type: "text" as const, text: "a" };
    const pic = { type: "image" as const, id: "1", width: 1, height: 1 };
    expect(mergeAttachments([a], [pic, a, { ...pic }])).toEqual([a, pic]);
  });
});

describe("composedQuestion", () => {
  it("uses what you typed, a prefix's AI, or the default question", () => {
    expect(composedQuestion("  fix this ")).toEqual({ text: "fix this", provider: null });
    expect(composedQuestion("g translate it")).toEqual({ text: "translate it", provider: "chatgpt" });
    expect(composedQuestion("")).toEqual({ text: DEFAULT_QUESTION, provider: null });
    expect(composedQuestion("   ")).toEqual({ text: DEFAULT_QUESTION, provider: null });
  });
});

describe("textPreview", () => {
  it("shows the first lines, then an ellipsis", () => {
    expect(textPreview("one\ntwo")).toBe("one\ntwo");
    expect(textPreview("1\n2\n3\n4", 2)).toBe("1\n2\n…");
    expect(textPreview("x".repeat(10), 8, 5)).toBe("xxxx…");
  });
});
