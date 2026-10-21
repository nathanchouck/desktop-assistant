import { describe, expect, it } from "vitest";
import { chatHints, chatIntentFor, intentFor, searchHints } from "./keys";

const key = (k: string, mods: Partial<{ ctrl: boolean; shift: boolean; alt: boolean }> = {}) => ({
  key: k,
  ctrlKey: mods.ctrl ?? false,
  shiftKey: mods.shift ?? false,
  altKey: mods.alt ?? false,
  metaKey: false,
});

describe("intentFor", () => {
  it("moves with arrows", () => {
    expect(intentFor(key("ArrowDown"))).toEqual({ type: "move", delta: 1 });
    expect(intentFor(key("ArrowUp"))).toEqual({ type: "move", delta: -1 });
  });

  it("maps Enter variants to actions", () => {
    expect(intentFor(key("Enter"))).toEqual({ type: "activate" });
    expect(intentFor(key("Enter", { shift: true }))).toEqual({ type: "activate" });
    expect(intentFor(key("Enter", { ctrl: true }))).toEqual({ type: "activate", action: "reveal_in_explorer" });
    expect(intentFor(key("Enter", { ctrl: true, shift: true }))).toEqual({
      type: "activate",
      action: "run_as_admin",
    });
  });

  it("copies the path with Ctrl+Shift+C", () => {
    expect(intentFor(key("C", { ctrl: true, shift: true }))).toEqual({ type: "activate", action: "copy_path" });
    expect(intentFor(key("c", { ctrl: true }))).toBeNull();
  });

  it("opens row N with Ctrl+N", () => {
    expect(intentFor(key("1", { ctrl: true }))).toEqual({ type: "activate", index: 0 });
    expect(intentFor(key("9", { ctrl: true }))).toEqual({ type: "activate", index: 8 });
  });

  it("leaves typing alone", () => {
    expect(intentFor(key("1"))).toBeNull();
    expect(intentFor(key("0", { ctrl: true }))).toBeNull();
    expect(intentFor(key("1", { ctrl: true, alt: true }))).toBeNull();
    expect(intentFor(key("a"))).toBeNull();
  });

  it("maps Escape", () => {
    expect(intentFor(key("Escape"))).toEqual({ type: "escape" });
  });

  it("asks the AI with Tab", () => {
    expect(intentFor(key("Tab"))).toEqual({ type: "ask" });
    expect(intentFor(key("Tab", { shift: true }))).toBeNull();
  });
});

describe("chatIntentFor", () => {
  it("sends with Enter only", () => {
    expect(chatIntentFor(key("Enter"))).toEqual({ type: "send" });
    expect(chatIntentFor(key("Enter", { shift: true }))).toBeNull();
    expect(chatIntentFor(key("Enter", { ctrl: true }))).toBeNull();
  });

  it("approves and denies with Ctrl+Y / Ctrl+N", () => {
    expect(chatIntentFor(key("y", { ctrl: true }))).toEqual({ type: "approve", allow: true });
    expect(chatIntentFor(key("N", { ctrl: true }))).toEqual({ type: "approve", allow: false });
    expect(chatIntentFor(key("y"))).toBeNull();
    expect(chatIntentFor(key("n", { ctrl: true, shift: true }))).toBeNull();
  });

  it("stops with Ctrl+. and scrolls with arrows and page keys", () => {
    expect(chatIntentFor(key(".", { ctrl: true }))).toEqual({ type: "stop" });
    expect(chatIntentFor(key("ArrowDown"))).toEqual({ type: "scroll", delta: 1, pages: false });
    expect(chatIntentFor(key("PageUp"))).toEqual({ type: "scroll", delta: -1, pages: true });
  });

  it("leaves typing alone", () => {
    expect(chatIntentFor(key("a"))).toBeNull();
    expect(chatIntentFor(key("Tab"))).toBeNull();
  });
});

describe("hints", () => {
  it("lists a result's extra actions", () => {
    expect(searchHints(["open", "reveal_in_explorer", "copy_path"]).map(([k]) => k)).toEqual([
      "↵",
      "Ctrl ↵",
      "Ctrl ⇧ C",
      "Esc",
    ]);
    expect(searchHints(["ask_ai"])[0]).toEqual(["↵", "Ask"]);
  });

  it("puts approvals first while chatting", () => {
    expect(chatHints({ running: true, pending: true })[0]).toEqual(["Ctrl Y", "Approve"]);
    expect(chatHints({ running: true, pending: false })).toEqual([["Esc", "Stop"]]);
    expect(chatHints({ running: false, pending: false })[0]).toEqual(["↵", "Send"]);
  });
});
