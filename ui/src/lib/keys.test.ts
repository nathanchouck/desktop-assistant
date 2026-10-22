import { describe, expect, it } from "vitest";
import { chatHints, chatIntentFor, intentFor, keyFor, searchHints, shortcutFromEvent } from "./keys";

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
    expect(intentFor(key("Enter", { ctrl: true }))).toEqual({
      type: "activate",
      actions: ["reveal_in_explorer", "paste"],
    });
    expect(intentFor(key("Enter", { ctrl: true, shift: true }))).toEqual({
      type: "activate",
      actions: ["run_as_admin"],
    });
  });

  it("copies the path with Ctrl+Shift+C", () => {
    expect(intentFor(key("C", { ctrl: true, shift: true }))).toEqual({
      type: "activate",
      actions: ["copy_path", "copy"],
    });
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

  it("opens settings with Ctrl+comma", () => {
    expect(intentFor(key(",", { ctrl: true }))).toEqual({ type: "settings" });
    expect(intentFor(key(","))).toBeNull();
  });

  it("asks the AI with Tab, the other one with Shift+Tab", () => {
    expect(intentFor(key("Tab"))).toEqual({ type: "ask" });
    expect(intentFor(key("Tab", { shift: true }))).toEqual({ type: "ask", other: true });
    expect(intentFor(key("Tab", { ctrl: true }))).toBeNull();
  });
});

describe("chatIntentFor", () => {
  it("sends with Enter only", () => {
    expect(chatIntentFor(key("Enter"))).toEqual({ type: "send" });
    expect(chatIntentFor(key("Enter", { shift: true }))).toBeNull();
  });

  it("pastes the last answer with Ctrl+Enter and copies it with Ctrl+Shift+C", () => {
    expect(chatIntentFor(key("Enter", { ctrl: true }))).toEqual({ type: "paste_answer" });
    expect(chatIntentFor(key("Enter", { ctrl: true, shift: true }))).toBeNull();
    expect(chatIntentFor(key("C", { ctrl: true, shift: true }))).toEqual({ type: "copy_answer" });
    expect(chatIntentFor(key("c", { ctrl: true, shift: true }))).toEqual({ type: "copy_answer" });
    // Plain Ctrl+C still copies the selection in the box.
    expect(chatIntentFor(key("c", { ctrl: true }))).toBeNull();
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

describe("action keys", () => {
  it("map to the actions they can mean", () => {
    expect(intentFor(key("P", { ctrl: true, shift: true }))).toEqual({ type: "activate", actions: ["pin"] });
    expect(intentFor(key("a", { ctrl: true, shift: true }))).toEqual({
      type: "activate",
      actions: ["ask_about"],
    });
    // Ctrl+D deletes a clip or chat, and sends a file to the Recycle Bin.
    expect(intentFor(key("d", { ctrl: true }))).toEqual({ type: "activate", actions: ["delete", "trash"] });
    expect(intentFor(key("d"))).toBeNull();
  });

  it("opens the action menu with Ctrl+K", () => {
    expect(intentFor(key("k", { ctrl: true }))).toEqual({ type: "menu" });
    expect(intentFor(key("K", { ctrl: true }))).toEqual({ type: "menu" });
    expect(intentFor(key("k"))).toBeNull();
    expect(intentFor(key("k", { ctrl: true, shift: true }))).toBeNull();
  });

  it("names the key that runs each action on a row", () => {
    const file = ["open", "reveal_in_explorer", "copy_path", "open_with", "trash"] as const;
    expect(keyFor([...file], "open")).toBe("↵");
    expect(keyFor([...file], "reveal_in_explorer")).toBe("Ctrl ↵");
    expect(keyFor([...file], "copy_path")).toBe("Ctrl ⇧ C");
    expect(keyFor([...file], "trash")).toBe("Ctrl D");
    // Only the menu has it.
    expect(keyFor([...file], "open_with")).toBeNull();
    // A key means the first of its actions the row offers.
    expect(keyFor(["copy", "paste"], "paste")).toBe("Ctrl ↵");
    expect(keyFor(["open", "reveal_in_explorer", "paste"], "paste")).toBeNull();
  });
});

describe("hints", () => {
  it("lists a result's extra actions", () => {
    expect(searchHints(["open", "reveal_in_explorer", "copy_path"]).map(([k]) => k)).toEqual([
      "↵",
      "Ctrl ↵",
      "Ctrl ⇧ C",
      "Ctrl K",
      "Esc",
    ]);
    // One action: nothing more to show in a menu.
    expect(searchHints(["run"])).toEqual([
      ["↵", "Run"],
      ["Esc", "Clear"],
    ]);
    expect(searchHints(["ask_ai"])[0]).toEqual(["↵", "Ask"]);
  });

  it("offers the other AI on the Ask row", () => {
    expect(searchHints(["ask_ai"], "ChatGPT")).toEqual([
      ["↵", "Ask"],
      ["⇧ Tab", "Ask ChatGPT"],
      ["Esc", "Clear"],
    ]);
    expect(searchHints(["open"], "ChatGPT").map(([k]) => k)).not.toContain("⇧ Tab");
  });

  it("names Enter after the default action and skips repeats of a key", () => {
    const hints = searchHints(["paste", "copy", "pin", "delete"]);
    expect(hints[0]).toEqual(["↵", "Paste"]);
    // Two extra keys at most; the menu (Ctrl+K) lists the rest.
    expect(hints.map(([k]) => k)).toEqual(["↵", "Ctrl ⇧ C", "Ctrl ⇧ P", "Ctrl K", "Esc"]);
    // Copy is the default: Ctrl+Enter pastes instead.
    expect(searchHints(["copy", "paste"]).map(([, l]) => l)).toEqual(["Copy", "Paste", "Actions", "Clear"]);
  });

  it("puts approvals first while chatting", () => {
    expect(chatHints({ running: true, pending: true })[0]).toEqual(["Ctrl Y", "Approve"]);
    expect(chatHints({ running: true, pending: false })).toEqual([["Esc", "Stop"]]);
    expect(chatHints({ running: false, pending: false })[0]).toEqual(["↵", "Send"]);
  });

  it("offers paste and copy once there's a finished answer", () => {
    expect(chatHints({ running: false, pending: false, answer: true })).toEqual([
      ["↵", "Send"],
      ["Ctrl ↵", "Paste"],
      ["Ctrl ⇧ C", "Copy"],
      ["Esc", "Close"],
    ]);
    expect(chatHints({ running: false, pending: false, answer: false })).toEqual([
      ["↵", "Send"],
      ["Esc", "Close"],
    ]);
    // Not while the next answer is coming or an approval waits.
    expect(chatHints({ running: true, pending: false, answer: true })).toEqual([["Esc", "Stop"]]);
    expect(chatHints({ running: true, pending: true, answer: true })[0]).toEqual(["Ctrl Y", "Approve"]);
  });
});

describe("shortcutFromEvent", () => {
  const press = (code: string, mods: Partial<{ ctrl: boolean; alt: boolean; shift: boolean; meta: boolean }> = {}) => ({
    code,
    ctrlKey: mods.ctrl ?? false,
    altKey: mods.alt ?? false,
    shiftKey: mods.shift ?? false,
    metaKey: mods.meta ?? false,
  });

  it("writes shortcuts the way the settings file does", () => {
    expect(shortcutFromEvent(press("Space", { ctrl: true }))).toBe("Ctrl+Space");
    expect(shortcutFromEvent(press("KeyK", { alt: true, shift: true }))).toBe("Alt+Shift+K");
    expect(shortcutFromEvent(press("Digit1", { ctrl: true }))).toBe("Ctrl+1");
    expect(shortcutFromEvent(press("F12"))).toBe("F12");
  });

  it("waits for a real key and needs a modifier", () => {
    expect(shortcutFromEvent(press("ControlLeft", { ctrl: true }))).toBeNull();
    expect(shortcutFromEvent(press("KeyK"))).toBeNull();
    expect(shortcutFromEvent(press("Space"))).toBeNull();
  });
});
