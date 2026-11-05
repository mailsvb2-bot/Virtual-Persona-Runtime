// Ported from #324; browser-only guard regression, NOT evidence of remote provider STOP.
import { expect, test } from "@playwright/test";

test("late microphone permission cannot reopen capture after unconfirmed STOP", async ({ page }) => {
  await page.addInitScript(() => {
    const win = window as typeof window & {
      LivekitClient?: unknown;
      __micPermissionRace?: { requested: number; stopped: number; contexts: number; release: () => void };
    };
    const race = { requested: 0, stopped: 0, contexts: 0, release: () => {} };
    win.__micPermissionRace = race;
    class FakeRoom {
      on(): FakeRoom { return this; }
      async connect(): Promise<void> {}
      async disconnect(): Promise<void> {}
    }
    win.LivekitClient = { Room: FakeRoom, RoomEvent: {} };
    Object.defineProperty(HTMLMediaElement.prototype, "play", { configurable: true, value: async () => {} });
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: {
        getUserMedia: () => {
          race.requested += 1;
          return new Promise((resolve) => {
            race.release = () => resolve({ getTracks: () => [{ stop: () => { race.stopped += 1; } }] });
          });
        },
      },
    });
    (win as unknown as { AudioContext: unknown }).AudioContext = class {
      constructor() { race.contexts += 1; }
    };
  });
  await page.route("**/api/bootstrap", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ csrf_token: "test" }) }));
  await page.route("**/api/rt1/visitor/start", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({
      session_id: "permission-race", voice_enabled: true,
      signal: { transport: { kind: "live_kit", server_url: "wss://example.test", token: "scoped" } },
    }) }));
  await page.route("**/api/rt1/visitor/status", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ active: true }) }));
  await page.route("**/api/rt1/visitor/close", (route) =>
    route.fulfill({ status: 502, contentType: "application/json", body: JSON.stringify({ code: "RT1_REMOTE_STOP_UNCONFIRMED" }) }));
  let voiceRequests = 0;
  await page.route("**/api/rt1/visitor/voice", (route) => {
    voiceRequests += 1;
    return route.fulfill({ status: 200, contentType: "application/json", body: "{}" });
  });
  await page.goto("/rt1");
  await page.locator("#visitor-token").fill("invite");
  await page.locator("#consent").check();
  await page.locator("#join").click();
  await expect(page.locator("#speak")).toBeEnabled();
  await page.locator("#speak").click();
  await expect.poll(() => page.evaluate(() => (
    (window as typeof window & { __micPermissionRace?: { requested: number } }).__micPermissionRace?.requested
  ))).toBe(1);
  await expect(page.locator("#speak")).toBeDisabled();
  // An unrelated busy() completion must not allow a second permission prompt.
  await page.locator("#unlock-audio").click();
  await expect(page.locator("#speak")).toBeDisabled();
  await expect.poll(() => page.evaluate(() => (
    (window as typeof window & { __micPermissionRace?: { requested: number } }).__micPermissionRace?.requested
  ))).toBe(1);
  await page.locator("#leave").click();
  await expect(page.locator("#status")).toContainText("RT1_REMOTE_STOP_UNCONFIRMED");
  await expect(page.locator("#leave")).toBeEnabled();
  await page.evaluate(() => (
    (window as typeof window & { __micPermissionRace?: { release: () => void } }).__micPermissionRace?.release()
  ));
  await expect.poll(() => page.evaluate(() => (
    (window as typeof window & { __micPermissionRace?: { stopped: number } }).__micPermissionRace?.stopped
  ))).toBe(1);
  const result = await page.evaluate(() => {
    const race = (window as typeof window & { __micPermissionRace?: { contexts: number; requested: number } }).__micPermissionRace;
    return { contexts: race?.contexts, requested: race?.requested };
  });
  expect(result).toEqual({ contexts: 0, requested: 1 });
  expect(voiceRequests).toBe(0);
  await expect(page.locator("#speak")).toBeDisabled();
});
