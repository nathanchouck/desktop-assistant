import { commands, type UpdateStatus } from "../bindings";
import { isNewer } from "../updates";

/** The settings menu's view of the updater: where it stands, Check now, Install & restart. */
export class UpdatesStore {
  status = $state<UpdateStatus | null>(null);
  /** The last button press failed (the installer couldn't start). */
  error = $state<string | null>(null);

  async load(): Promise<void> {
    this.apply(await commands.updateStatus());
  }

  /** A status from a reply or an `update-status-changed` event; older ones are ignored. */
  apply(status: UpdateStatus): void {
    if (isNewer(status, this.status)) this.status = status;
  }

  /** Checks now; a newer version starts downloading (progress comes as events). */
  async check(): Promise<void> {
    this.error = null;
    const res = await commands.updateCheck();
    if (res.status === "ok") this.apply(res.data);
    else this.error = res.error.message;
  }

  /** Installs the downloaded update; the app closes and the new version opens. */
  async install(): Promise<void> {
    this.error = null;
    const res = await commands.updateInstall();
    if (res.status === "error") this.error = res.error.message;
  }
}
