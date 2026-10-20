import { describe, expect, it } from "vitest";
import { LatestOnly } from "./latest";

describe("LatestOnly", () => {
  it("applies responses in order", () => {
    const g = new LatestOnly();
    const a = g.next();
    const b = g.next();
    expect(g.accept(a)).toBe(true);
    expect(g.accept(b)).toBe(true);
  });

  it("drops a response that arrives after a newer one", () => {
    const g = new LatestOnly();
    const a = g.next();
    const b = g.next();
    expect(g.accept(b)).toBe(true);
    expect(g.accept(a)).toBe(false);
  });

  it("drops duplicates and never-issued sequence numbers", () => {
    const g = new LatestOnly();
    const a = g.next();
    expect(g.accept(a)).toBe(true);
    expect(g.accept(a)).toBe(false);
    expect(g.accept(99)).toBe(false);
  });
});
