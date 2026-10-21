import type { Toast } from "../bindings";

/** How long a toast stays in the footer. Errors stay longer. */
export const TOAST_MS = { info: 4000, error: 8000 } as const;

/** The footer's toast: the latest message from the backend, for a while. */
export class ToastStore {
  current = $state<Toast | null>(null);
  #timer: ReturnType<typeof setTimeout> | undefined;

  show(toast: Toast): void {
    this.current = toast;
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => this.dismiss(), TOAST_MS[toast.level]);
  }

  dismiss(): void {
    clearTimeout(this.#timer);
    this.current = null;
  }
}
