// Ported from #324: deterministic browser lifecycle test; not a remote D-ID STOP receipt.
import { expect, test } from "@playwright/test";

test("RT1 disconnects a LiveKit connection that completes after pagehide revoke", async ({ page }) => {
  await page.addInitScript(() => {
    const win = window as typeof window & {
      LivekitClient?: unknown;
      __rt1LateConnect?: { ready: boolean; disconnects: number; release: () => void };
    };
    const state = { ready: false, disconnects: 0, release: () => {} };
    win.__rt1LateConnect = state;
    class DelayedRoom {
      on(): DelayedRoom { return this; }
      connect(): Promise<void> {
        state.ready = true;
        return new Promise<void>((resolve) => { state.release = resolve; });
      }
      async disconnect(): Promise<void> { state.disconnects += 1; }
    }
    win.LivekitClient = {
      Room: DelayedRoom,
      RoomEvent: {
        TrackSubscribed: "subscribed",
        TrackUnsubscribed: "unsubscribed",
        Disconnected: "disconnected",
      },
    };
  });

  await page.route("**/api/bootstrap", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ csrf_token: "test" }) }));
  await page.route("**/api/rt1/visitor/start", (route) =>
    route.fulfill({
      status: 200, contentType: "application/json",
      body: JSON.stringify({
        session_id: "late-livekit",
        signal: { transport: { kind: "live_kit", server_url: "wss://example.test", token: "scoped" } },
      }),
    }));
  await page.route("**/api/rt1/visitor/close", (route) =>
    route.fulfill({
      status: 200, contentType: "application/json",
      body: JSON.stringify({ ok: true, remote_stop_confirmed: true }),
    }));

  await page.goto("/rt1");
  await page.locator("#visitor-token").fill("visitor-invite");
  await page.locator("#consent").check();
  await page.locator("#join").click();
  await expect.poll(() => page.evaluate(() => (
    (window as typeof window & { __rt1LateConnect?: { ready: boolean } }).__rt1LateConnect?.ready
  ))).toBe(true);

  await page.evaluate(() => window.dispatchEvent(new Event("pagehide")));
  await expect.poll(() => page.evaluate(() => (
    (window as typeof window & { __rt1LateConnect?: { disconnects: number } }).__rt1LateConnect?.disconnects
  ))).toBe(1);

  await page.evaluate(() => (
    (window as typeof window & { __rt1LateConnect?: { release: () => void } }).__rt1LateConnect?.release()
  ));
  await expect.poll(() => page.evaluate(() => (
    (window as typeof window & { __rt1LateConnect?: { disconnects: number } }).__rt1LateConnect?.disconnects
  ))).toBe(2);
});
