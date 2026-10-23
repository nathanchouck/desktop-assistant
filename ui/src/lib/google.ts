import type { GoogleSettings, GoogleStatus } from "./bindings";

/** Where the OAuth client is made. */
export const CLOUD_CONSOLE = "https://console.cloud.google.com/";

/** How many days `cal` can cover. */
export const MIN_DAYS = 1;
export const MAX_DAYS = 7;

/** The account line in Settings → Google. */
export function statusLine(status: GoogleStatus | null): string {
  if (!status) return "Checking…";
  if (status.connecting) return "Waiting for you to allow access in the browser…";
  if (status.connected) return status.email ? `Connected as ${status.email}` : "Connected";
  return "Not connected";
}

/** Why "Connect" can't run yet, or null when it can. */
export function connectBlocker(settings: GoogleSettings, status: GoogleStatus | null): string | null {
  if (!settings.client_id.trim()) return "Enter the client ID first.";
  if (!status?.secret_saved) return "Save the client secret first.";
  return null;
}

const CLIENT_ID_END = ".apps.googleusercontent.com";

/** A warning when the text pasted as the client ID doesn't look like one. */
export function clientIdWarning(id: string): string | null {
  const trimmed = id.trim();
  if (!trimmed || trimmed.endsWith(CLIENT_ID_END)) return null;
  if (trimmed.startsWith("GOCSPX-")) return "That's the client secret; it goes in the field below.";
  return `A client ID ends in ${CLIENT_ID_END}.`;
}

/** The setup steps, for an OAuth client of your own. */
export const SETUP_STEPS: string[] = [
  "Open Google Cloud Console and create a project (any name).",
  "APIs & Services → Library: enable the Google Calendar API, Gmail API and Google Drive API.",
  "Google Auth Platform (OAuth consent screen): Get started, choose External, then under Audience add your own Google address as a test user.",
  "Clients → Create client → Application type “Desktop app”. Paste its client ID and client secret here.",
  "Press Connect and allow access in the browser. Access is read-only: events, mail subjects and senders (never bodies), Drive file names.",
];
