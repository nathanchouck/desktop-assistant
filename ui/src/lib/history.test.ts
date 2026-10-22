import { describe, expect, it } from "vitest";
import { clearable, forgetOptions, historySummary } from "./history";

describe("forgetOptions", () => {
  it("offers never and a few common ages", () => {
    expect(forgetOptions(0)).toEqual([
      ["0", "Never"],
      ["30", "30 days"],
      ["90", "90 days"],
      ["365", "1 year"],
    ]);
  });

  it("lists a hand-set age in order, after Never", () => {
    expect(forgetOptions(14).map(([v]) => v)).toEqual(["0", "14", "30", "90", "365"]);
    expect(forgetOptions(14)[1]).toEqual(["14", "14 days"]);
    expect(forgetOptions(1)[1]).toEqual(["1", "1 day"]);
    expect(forgetOptions(400).at(-1)).toEqual(["400", "400 days"]);
  });
});

describe("historySummary", () => {
  it("counts saved and pinned chats", () => {
    expect(historySummary({ conversations: 0, pinned: 0 })).toBe("No saved chats.");
    expect(historySummary({ conversations: 1, pinned: 0 })).toBe("1 chat saved.");
    expect(historySummary({ conversations: 12, pinned: 2 })).toBe("12 chats saved, 2 pinned.");
  });

  it("knows what Clear would remove", () => {
    expect(clearable({ conversations: 12, pinned: 2 })).toBe(10);
    expect(clearable({ conversations: 2, pinned: 2 })).toBe(0);
  });
});
