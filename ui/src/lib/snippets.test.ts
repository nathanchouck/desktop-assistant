import { describe, expect, it } from "vitest";
import type { Snippet } from "./bindings";
import { expandsAsYouType, snippetWarnings } from "./snippets";

const snip = (name: string, keyword: string): Snippet => ({ name, keyword, text: "" });

describe("expandsAsYouType", () => {
  it("needs the trigger and something after it", () => {
    expect(expandsAsYouType(";sig", ";")).toBe(true);
    expect(expandsAsYouType(" ;sig ", " ; ")).toBe(true);
    expect(expandsAsYouType("sig", ";")).toBe(false);
    expect(expandsAsYouType(";", ";")).toBe(false);
    expect(expandsAsYouType("sig", "")).toBe(true);
    expect(expandsAsYouType("", "")).toBe(false);
  });
});

describe("snippetWarnings", () => {
  it("is quiet for good keywords", () => {
    const items = [snip("Sig", ";sig"), snip("Addr", ";addr"), snip("No keyword", "")];
    expect(snippetWarnings(items, ";", true)).toEqual([[], [], []]);
  });

  it("flags the same keyword twice, ignoring case", () => {
    const warnings = snippetWarnings([snip("One", ";a"), snip("Two", ";A")], ";", false);
    expect(warnings[0]).toEqual(["Same keyword as “Two”."]);
    expect(warnings[1]).toEqual(["Same keyword as “One”."]);
  });

  it("flags keywords without the trigger only while expanding", () => {
    expect(snippetWarnings([snip("Sig", "sig")], ";", false)).toEqual([[]]);
    expect(snippetWarnings([snip("Sig", "sig")], ";", true)[0]?.[0]).toMatch(/only works from search/);
  });

  it("flags keywords a shorter one cuts off", () => {
    const warnings = snippetWarnings([snip("Short", ";s"), snip("Long", ";sig")], ";", true);
    expect(warnings[0]).toEqual([]);
    expect(warnings[1]).toEqual(["Never expands: “;s” expands first while you type it."]);
    // A keyword that only ends where another ends isn't cut off (the longer wins).
    expect(snippetWarnings([snip("A", "ig"), snip("B", ";sig")], "", true)).toEqual([[], []]);
    // Off: nothing expands, so nothing is cut off.
    expect(snippetWarnings([snip("Short", ";s"), snip("Long", ";sig")], ";", false)).toEqual([[], []]);
  });
});
