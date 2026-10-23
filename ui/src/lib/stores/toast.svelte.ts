import type { Toast } from "../bindings";

/** How long a toast stays in the footer. Errors stay longer. */
export const TOAST_MS = { info: 4000, error: 8000 } as const;
/**
 * A toast that comes while the launcher is hidden waits for it to open,
 * if that's within this long (Rust keeps toasts for a late UI as long).
 */
export const PENDING_MS = 2 * 60 * 1000;

/**
 * The footer's toast: the latest message from the backend, for a while.
 * While the launcher is hidden nobody can read it, so it waits (already
 * in place, so the window doesn't change size as it opens) and its time
 * starts when the launcher opens.
 */
export class ToastStore {
  current = $state<Toast | null>(null);
  #hidden = false;
  /** When `current` arrived, if it's waiting for the launcher to open. */
  #waitingSince: number | null = null;
  #timer: ReturnType<typeof setTimeout> | undefined;

  /** A toast sent at `at` (now, or earlier for one sent before the UI listened). */
  show(toast: Toast, at = Date.now()): void {
    if (this.#hidden) {
      // A waiting error isn't pushed out by news ("Copied …").
      const waiting = this.#waitingSince !== null ? this.current : null;
      if (waiting?.level === "error" && toast.level === "info") return;
      this.#wait(toast, at);
      return;
    }
    this.#display(toast);
  }

  /** The launcher hid: what arrives from now on waits for it to open. */
  hidden(): void {
    this.#hidden = true;
  }

  /** The launcher opened: a waiting toast shows now, if it's still recent. */
  shown(now = Date.now()): void {
    this.#hidden = false;
    const since = this.#waitingSince;
    if (since === null || !this.current) return;
    if (now - since > PENDING_MS) this.dismiss();
    else this.#display(this.current);
  }

  dismiss(): void {
    clearTimeout(this.#timer);
    this.current = null;
    this.#waitingSince = null;
  }

  #display(toast: Toast): void {
    clearTimeout(this.#timer);
    this.current = toast;
    this.#waitingSince = null;
    this.#timer = setTimeout(() => this.dismiss(), TOAST_MS[toast.level]);
  }

  #wait(toast: Toast, at: number): void {
    clearTimeout(this.#timer);
    const left = at + PENDING_MS - Date.now();
    if (left <= 0) return;
    this.current = toast;
    this.#waitingSince = at;
    // Too old by the time the launcher opens: gone while still hidden.
    this.#timer = setTimeout(() => this.dismiss(), left);
  }
}
