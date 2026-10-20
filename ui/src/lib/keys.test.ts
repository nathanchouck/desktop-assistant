import { describe, expect, it } from "vitest";
import { intentFor } from "./keys";

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

  it("opens with Enter and runs as admin with Ctrl+Shift+Enter", () => {
    expect(intentFor(key("Enter"))).toEqual({ type: "activate", admin: false });
    expect(intentFor(key("Enter", { ctrl: true, shift: true }))).toEqual({ type: "activate", admin: true });
    expect(intentFor(key("Enter", { shift: true }))).toEqual({ type: "activate", admin: false });
  });

  it("opens row N with Ctrl+N", () => {
    expect(intentFor(key("1", { ctrl: true }))).toEqual({ type: "activate", index: 0, admin: false });
    expect(intentFor(key("9", { ctrl: true }))).toEqual({ type: "activate", index: 8, admin: false });
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
});
