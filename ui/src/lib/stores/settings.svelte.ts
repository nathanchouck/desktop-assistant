import { commands, type AppStatus, type Settings } from "../bindings";

/** The settings menu: current settings, saving, and the status lines. */
export class SettingsStore {
  settings = $state<Settings | null>(null);
  status = $state<AppStatus | null>(null);
  /** The last save failed (e.g. a shortcut another app already uses). */
  error = $state<string | null>(null);
  /** Folder or exclusion changes wait for a restart. */
  restartNeeded = $state(false);

  async load(): Promise<void> {
    this.settings = await commands.settingsGet();
    this.error = null;
    await this.refreshStatus();
  }

  async refreshStatus(): Promise<void> {
    const res = await commands.appStatus();
    if (res.status === "ok") this.status = res.data;
  }

  /** Applies `change` to a copy and saves it; the menu updates once saved. */
  async save(change: (next: Settings) => void): Promise<boolean> {
    if (!this.settings) return false;
    const next = structuredClone($state.snapshot(this.settings)) as Settings;
    change(next);
    const res = await commands.settingsSet(next);
    if (res.status === "error") {
      this.error = res.error.message;
      return false;
    }
    this.settings = next;
    this.error = null;
    this.restartNeeded = res.data.restart_needed;
    void this.refreshStatus();
    return true;
  }
}
