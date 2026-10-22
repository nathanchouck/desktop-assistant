import { beforeEach, describe, expect, it, vi } from "vitest";
import type { UpdatePhase, UpdateStatus } from "../bindings";

const commands = vi.hoisted(() => ({
  updateStatus: vi.fn(),
  updateCheck: vi.fn(),
  updateInstall: vi.fn(),
}));
vi.mock("../bindings", () => ({ commands }));

const { UpdatesStore } = await import("./updates.svelte");

const status = (phase: UpdatePhase, revision: number): UpdateStatus => ({
  current_version: "0.1.0",
  phase,
  summary: phase.state,
  last_checked: null,
  revision,
});

describe("UpdatesStore", () => {
  beforeEach(() => vi.resetAllMocks());

  it("loads the status and follows newer events", async () => {
    commands.updateStatus.mockResolvedValueOnce(status({ state: "idle" }, 1));
    const store = new UpdatesStore();
    await store.load();
    expect(store.status?.phase.state).toBe("idle");
    store.apply(status({ state: "checking" }, 2));
    expect(store.status?.phase.state).toBe("checking");
  });

  it("ignores a check reply that arrives after newer events", async () => {
    const store = new UpdatesStore();
    store.apply(status({ state: "idle" }, 1));
    let reply: (value: unknown) => void = () => {};
    commands.updateCheck.mockReturnValueOnce(new Promise((resolve) => (reply = resolve)));
    const checking = store.check();
    // The download finished before the check's reply came back.
    store.apply(status({ state: "ready", version: "0.2.0" }, 5));
    reply({ status: "ok", data: status({ state: "downloading", version: "0.2.0", percent: null }, 3) });
    await checking;
    expect(store.status?.phase).toEqual({ state: "ready", version: "0.2.0" });
    expect(store.error).toBeNull();
  });

  it("shows why an install couldn't start, and clears it on the next try", async () => {
    const store = new UpdatesStore();
    commands.updateInstall.mockResolvedValueOnce({ status: "error", error: { message: "Couldn't install the update: no update has been downloaded" } });
    await store.install();
    expect(store.error).toContain("no update has been downloaded");
    commands.updateCheck.mockResolvedValueOnce({ status: "ok", data: status({ state: "up_to_date" }, 2) });
    await store.check();
    expect(store.error).toBeNull();
    expect(store.status?.phase.state).toBe("up_to_date");
  });
});
