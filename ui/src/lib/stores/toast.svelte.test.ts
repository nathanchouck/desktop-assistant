import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TOAST_MS, ToastStore } from "./toast.svelte";

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
});
