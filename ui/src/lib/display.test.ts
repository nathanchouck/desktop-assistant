import { describe, expect, it } from "vitest";
import { indexNotice, shortenPath } from "./display";

describe("shortenPath", () => {
  it("leaves short paths alone", () => {
    expect(shortenPath("~\\Documents")).toBe("~\\Documents");
  });

  it("cuts folders out of the middle", () => {
    const long = "C:\\Users\\nathan\\Documents\\School\\Spring 2026\\Astrodynamics\\Homework";
    const short = shortenPath(long, 40);
    expect(short.startsWith("C:\\…\\")).toBe(true);
    expect(short.endsWith("\\Homework")).toBe(true);
    expect(short.length).toBeLessThanOrEqual(40);
  });

  it("trims the start when there are no folders to drop", () => {
    const long = "D:\\" + "x".repeat(80);
    const short = shortenPath(long, 20);
    expect(short.startsWith("…")).toBe(true);
    expect(short.length).toBe(20);
  });

  it("handles forward slashes", () => {
    const short = shortenPath("/home/n/a/b/c/d/e/f/g/h", 14);
    expect(short.startsWith("/…/")).toBe(true);
    expect(short.endsWith("/g/h")).toBe(true);
    expect(short.length).toBeLessThanOrEqual(14);
  });
});

describe("indexNotice", () => {
  it("shows progress only while files may be missing", () => {
    expect(indexNotice(null)).toBeNull();
    expect(indexNotice({ scanning: true, incomplete: false, entries: 9 })).toBeNull();
    expect(indexNotice({ scanning: false, incomplete: true, entries: 0 })).toBe("Loading your file index…");
    expect(indexNotice({ scanning: true, incomplete: true, entries: 48213 })).toBe(
      `Indexing your files… ${(48213).toLocaleString()} found so far`,
    );
  });
});
