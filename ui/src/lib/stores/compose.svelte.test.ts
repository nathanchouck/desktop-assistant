import { describe, expect, it } from "vitest";
import type { Attachment } from "../bindings";
import { MAX_ATTACHMENTS } from "../compose";
import { ComposeStore } from "./compose.svelte";

const text = (t: string): Attachment => ({ type: "text", text: t });
const file = (path: string): Attachment => ({ type: "path", path, folder: false });

describe("ComposeStore", () => {
  it("is active while it has attachments", () => {
    const compose = new ComposeStore();
    expect(compose.active).toBe(false);
    compose.add([text("teh cat")]);
    expect(compose.active).toBe(true);
  });

  it("skips attachments it already has", () => {
    const compose = new ComposeStore();
    compose.add([file("C:\\a.txt"), text("x")]);
    compose.add([file("C:\\a.txt"), file("C:\\b.txt")]);
    expect(compose.attachments).toEqual([file("C:\\a.txt"), text("x"), file("C:\\b.txt")]);
    compose.add(Array.from({ length: 20 }, (_, i) => file(`C:\\${i}`)));
    expect(compose.attachments).toHaveLength(MAX_ATTACHMENTS);
  });

  it("drops the last chip, and the question with the last one", () => {
    const compose = new ComposeStore();
    compose.add([text("a"), file("C:\\b")]);
    compose.draft = "why";
    compose.removeLast();
    expect(compose.attachments).toEqual([text("a")]);
    expect(compose.draft).toBe("why");
    compose.remove(0);
    expect(compose.active).toBe(false);
    expect(compose.draft).toBe("");
  });

  it("hands its attachments over once", () => {
    const compose = new ComposeStore();
    compose.add([text("a")]);
    compose.draft = "fix it";
    expect(compose.take()).toEqual([text("a")]);
    expect(compose.active).toBe(false);
    expect(compose.draft).toBe("");
    expect(compose.take()).toEqual([]);
  });
});
