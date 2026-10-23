import { describe, expect, it } from "vitest";
import type { ResultItem } from "../bindings";
import { ActionMenuStore } from "./actionMenu.svelte";

const clip: ResultItem = {
  id: "clip:1",
  kind: "clip",
  title: "hello",
  subtitle: "",
  icon: null,
  actions: ["paste", "copy", "pin", "delete"],
  accessory: null,
  confirm: null,
};

describe("ActionMenuStore", () => {
  it("opens on a row and lists its actions", () => {
    const menu = new ActionMenuStore();
    expect(menu.open).toBe(false);
    menu.show(clip);
    expect(menu.open).toBe(true);
    expect(menu.entries.map((e) => e.action)).toEqual(["paste", "copy", "pin", "delete"]);
  });

  it("moves around the list, wrapping", () => {
    const menu = new ActionMenuStore();
    menu.show(clip);
    menu.move(-1);
    expect(menu.selected).toBe(3);
    menu.move(1);
    expect(menu.selected).toBe(0);
  });

  it("filters as you type and starts again at the top", () => {
    const menu = new ActionMenuStore();
    menu.show(clip);
    menu.move(2);
    menu.setFilter("de");
    expect(menu.selected).toBe(0);
    expect(menu.entries.map((e) => e.action)).toEqual(["delete"]);
    menu.setFilter("zzz");
    menu.move(1);
    expect(menu.selected).toBe(0);
    expect(menu.choose()).toBeNull();
    expect(menu.open).toBe(true);
  });

  it("hands back the chosen action and closes", () => {
    const menu = new ActionMenuStore();
    menu.show(clip);
    menu.move(1);
    expect(menu.choose()).toBe("copy");
    expect(menu.open).toBe(false);
    menu.show(clip);
    expect(menu.filter).toBe("");
    expect(menu.choose(2)).toBe("pin");
  });
});
