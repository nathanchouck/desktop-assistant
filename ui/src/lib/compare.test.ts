import { describe, expect, it } from "vitest";
import type { Transcript } from "./chat";
import { compareNotice, comparePrefix, sideStatus } from "./compare";

const transcript = (t: Partial<Transcript>): Transcript => ({ entries: [], running: false, model: "", ...t });

describe("comparePrefix", () => {
  it("takes 'both' with any question", () => {
    expect(comparePrefix("both which is faster?")).toBe("which is faster?");
    expect(comparePrefix("  Both  hi ")).toBe("hi");
    expect(comparePrefix("both ")).toBeNull();
    expect(comparePrefix("bothell weather")).toBeNull();
  });

  it("takes 'vs' only from three words on, so VS Code still opens", () => {
    expect(comparePrefix("vs explain this error")).toBe("explain this error");
    expect(comparePrefix("vs code")).toBeNull();
    expect(comparePrefix("vs code insiders")).toBeNull();
  });
});

describe("sideStatus", () => {
  it("tells thinking, answering and waiting apart while running", () => {
    expect(sideStatus(transcript({ running: true, entries: [{ kind: "user", text: "q" }] }))).toBe("thinking");
    expect(
      sideStatus(
        transcript({
          running: true,
          entries: [
            { kind: "user", text: "q" },
            { kind: "assistant", text: "Well" },
          ],
        }),
      ),
    ).toBe("answering");
    expect(
      sideStatus(
        transcript({
          running: true,
          entries: [{ kind: "approval", id: "1", tool: "Bash", summary: "", detail: "", state: "pending" }],
        }),
      ),
    ).toBe("approval");
  });

  it("tells how a turn ended", () => {
    expect(sideStatus(transcript({ entries: [{ kind: "assistant", text: "4" }] }))).toBe("done");
    expect(sideStatus(transcript({ entries: [{ kind: "note", text: "Stopped" }] }))).toBe("stopped");
    expect(sideStatus(transcript({ entries: [{ kind: "error", text: "signed out" }] }))).toBe("error");
  });
});

describe("compareNotice", () => {
  it("names each AI and model with how it's doing", () => {
    expect(
      compareNotice([
        { provider: "claude", transcript: transcript({ model: "claude-sonnet-5-5" }) },
        { provider: "chatgpt", transcript: transcript({ model: "gpt-6.1-sol", running: true }) },
      ]),
    ).toBe("Claude · Sonnet 5.5 · done | ChatGPT · GPT-6.1-Sol · thinking");
    expect(compareNotice([{ provider: "chatgpt", transcript: transcript({ running: true }) }])).toBe(
      "ChatGPT · thinking",
    );
  });
});
