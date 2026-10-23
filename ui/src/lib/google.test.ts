import { describe, expect, it } from "vitest";
import type { GoogleSettings, GoogleStatus } from "./bindings";
import { clientIdWarning, connectBlocker, statusLine } from "./google";

const status = (over: Partial<GoogleStatus> = {}): GoogleStatus => ({
  secret_saved: false,
  connecting: false,
  connected: false,
  email: null,
  problem: null,
  ...over,
});

const settings = (client_id: string): GoogleSettings => ({
  enabled: true,
  client_id,
  drive_in_search: true,
  calendar_days: 2,
});

describe("statusLine", () => {
  it("says who's connected", () => {
    expect(statusLine(status({ connected: true, email: "me@example.com" }))).toBe("Connected as me@example.com");
    expect(statusLine(status({ connected: true }))).toBe("Connected");
  });

  it("says when it's waiting or not connected", () => {
    expect(statusLine(null)).toBe("Checking…");
    expect(statusLine(status({ connecting: true }))).toBe("Waiting for you to allow access in the browser…");
    expect(statusLine(status())).toBe("Not connected");
  });
});

describe("connectBlocker", () => {
  it("needs the client ID, then the secret", () => {
    expect(connectBlocker(settings(" "), status({ secret_saved: true }))).toBe("Enter the client ID first.");
    expect(connectBlocker(settings("1.apps.googleusercontent.com"), status())).toBe("Save the client secret first.");
    expect(connectBlocker(settings("1.apps.googleusercontent.com"), null)).toBe("Save the client secret first.");
    expect(connectBlocker(settings("1.apps.googleusercontent.com"), status({ secret_saved: true }))).toBeNull();
  });
});

describe("clientIdWarning", () => {
  it("accepts client IDs and catches a pasted secret", () => {
    expect(clientIdWarning("")).toBeNull();
    expect(clientIdWarning(" 123-abc.apps.googleusercontent.com ")).toBeNull();
    expect(clientIdWarning("GOCSPX-abc")).toBe("That's the client secret; it goes in the field below.");
    expect(clientIdWarning("123")).toBe("A client ID ends in .apps.googleusercontent.com.");
  });
});
