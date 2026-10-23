import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PENDING_MS, TOAST_MS, ToastStore } from "./toast.svelte";

describe("ToastStore", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("shows the latest toast and clears it after a while", () => {
    const toasts = new ToastStore();
    toasts.show({ level: "info", message: "first" });
    toasts.show({ level: "error", message: "second" });
    expect(toasts.current?.message).toBe("second");
    vi.advanceTimersByTime(TOAST_MS.info + 1);
    expect(toasts.current?.message).toBe("second");
    vi.advanceTimersByTime(TOAST_MS.error);
    expect(toasts.current).toBeNull();
  });

  it("can be dismissed early", () => {
    const toasts = new ToastStore();
    toasts.show({ level: "info", message: "hi" });
    toasts.dismiss();
    expect(toasts.current).toBeNull();
  });

  it("holds a toast that came while hidden until the launcher opens", () => {
    const toasts = new ToastStore();
    toasts.hidden();
    toasts.show({ level: "error", message: "index failed" });
    // Its time doesn't run out while nobody can see it...
    vi.advanceTimersByTime(TOAST_MS.error + 1000);
    expect(toasts.current?.message).toBe("index failed");
    // ...it starts when the launcher opens.
    toasts.shown();
    vi.advanceTimersByTime(TOAST_MS.error - 1);
    expect(toasts.current?.message).toBe("index failed");
    vi.advanceTimersByTime(2);
    expect(toasts.current).toBeNull();
  });

  it("drops a waiting toast once it's too old to matter", () => {
    const toasts = new ToastStore();
    toasts.hidden();
    toasts.show({ level: "error", message: "old news" });
    vi.advanceTimersByTime(PENDING_MS + 1);
    expect(toasts.current).toBeNull();
    toasts.shown();
    expect(toasts.current).toBeNull();
  });

  it("doesn't show one that was already too old when it opens", () => {
    const toasts = new ToastStore();
    toasts.hidden();
    const start = Date.now();
    toasts.show({ level: "error", message: "late" }, start);
    // Timers can run late while the launcher is hidden; the age decides.
    toasts.shown(start + PENDING_MS + 1);
    expect(toasts.current).toBeNull();
  });

  it("takes toasts sent before the UI listened, by their age", () => {
    const toasts = new ToastStore();
    toasts.hidden();
    toasts.show({ level: "error", message: "too old" }, Date.now() - PENDING_MS - 1);
    expect(toasts.current).toBeNull();
    toasts.show({ level: "error", message: "settings broken" }, Date.now() - 30_000);
    expect(toasts.current?.message).toBe("settings broken");
    vi.advanceTimersByTime(PENDING_MS - 30_000 + 1);
    expect(toasts.current).toBeNull();
  });

  it("keeps a waiting error over later news", () => {
    const toasts = new ToastStore();
    toasts.hidden();
    toasts.show({ level: "error", message: "hook failed" });
    toasts.show({ level: "info", message: "Copied #ff8800" });
    expect(toasts.current?.message).toBe("hook failed");
    toasts.show({ level: "error", message: "index failed" });
    expect(toasts.current?.message).toBe("index failed");
    // While open, the newest always shows.
    toasts.shown();
    toasts.show({ level: "info", message: "Copied" });
    expect(toasts.current?.message).toBe("Copied");
  });

  it("lets a toast that was on screen run out after the launcher hides", () => {
    const toasts = new ToastStore();
    toasts.show({ level: "info", message: "seen" });
    toasts.hidden();
    vi.advanceTimersByTime(TOAST_MS.info + 1);
    expect(toasts.current).toBeNull();
    toasts.shown();
    expect(toasts.current).toBeNull();
  });
});
