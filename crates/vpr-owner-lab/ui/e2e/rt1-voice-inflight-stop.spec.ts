// Selected from #324: browser lifecycle regression, not provider STOP proof.
import { expect, test } from "@playwright/test";

test("visitor microphone stays disabled during pending voice and unconfirmed remote STOP", async ({ page }) => {
  await page.addInitScript(() => {
    const win = window as typeof window & { LivekitClient?: unknown; __fakeProcessor?: { port: { onmessage: ((event: {data: Float32Array}) => void) | null } } };
    class FakeRoom {
      on(): FakeRoom { return this; }
      async connect(): Promise<void> {}
      async disconnect(): Promise<void> {}
    }
    win.LivekitClient = { Room: FakeRoom, RoomEvent: {} };
    // Make the unrelated audio-unlock action deterministic and awaitable.
    Object.defineProperty(HTMLMediaElement.prototype, "play", { configurable: true, value: async () => {} });
    const navigatorWithMedia = navigator as Navigator & { mediaDevices: { getUserMedia: () => Promise<unknown> } };
    Object.defineProperty(navigatorWithMedia, "mediaDevices", {
      configurable: true,
      value: { getUserMedia: async () => ({ getTracks: () => [{ stop: () => {} }] }) },
    });
    class FakeContext {
      sampleRate = 16000;
      destination = {};
      audioWorklet = { addModule: async () => {} };
      createMediaStreamSource(): { connect: () => void; disconnect: () => void } {
        return { connect: () => {}, disconnect: () => {} };
      }
      async close(): Promise<void> {}
    }
    (win as unknown as { AudioContext: unknown }).AudioContext = FakeContext;
    (win as unknown as { AudioWorkletNode: unknown }).AudioWorkletNode = class {
      port = { onmessage: null as ((event: {data: Float32Array}) => void) | null };
      constructor() { win.__fakeProcessor = this; }
      connect(): void {}
      disconnect(): void {}
    };
  });
  await page.route("**/api/bootstrap", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({csrf_token: "test"}) }));
  await page.route("**/api/rt1/visitor/start", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({
      session_id: "voice-visitor", voice_enabled: true,
      signal: {transport: {kind: "live_kit", server_url: "wss://example.test", token: "scoped"}},
    }) }));
  let voiceRequests = 0;
  let interrupts = 0;
  await page.route("**/api/rt1/visitor/interrupt", (route) => {
    interrupts += 1;
    return route.fulfill({
      status: 200, contentType: "application/json",
      body: JSON.stringify({ ok: true, remote_video_stop_confirmed: false }),
    });
  });
  let release: (() => void) | null = null;
  await page.route("**/api/rt1/visitor/voice", async (route) => {
    voiceRequests += 1;
    await new Promise<void>((resolve) => { release = resolve; });
    await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({transcript: "Привет", reply: "Здравствуйте"}) });
  });
  await page.route("**/api/rt1/visitor/status", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({active: true}) }));
  await page.route("**/api/rt1/visitor/close", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ok: true, remote_stop_confirmed: true}) }));

  await page.goto("/rt1");
  await page.locator("#visitor-token").fill("invite");
  await page.locator("#consent").check();
  await page.locator("#join").click();
  await expect(page.locator("#speak")).toBeEnabled();
  await page.locator("#speak").click();
  await expect(page.locator("#speak")).toHaveText("Отправить речь");
  await page.evaluate(() => {
    const win = window as typeof window & { __fakeProcessor?: {port: {onmessage: ((event: {data: Float32Array}) => void) | null}} };
    win.__fakeProcessor?.port.onmessage?.({data: new Float32Array([0.1, 0.2])});
  });
  await page.locator("#speak").click();
  await expect.poll(() => voiceRequests).toBe(1);
  await expect(page.locator("#speak")).toBeDisabled();
  // A separate busy() action must not override the pending voice-turn lock.
  await page.locator("#unlock-audio").click();
  await expect(page.locator("#status")).toContainText("Воспроизведение звука разрешено");
  await expect(page.locator("#speak")).toBeDisabled();
  await page.locator("#interrupt").click();
  await expect.poll(() => interrupts).toBe(1);
  await expect(page.locator("#status")).toContainText("Ответ прерван");
  (release as (() => void) | null)?.();
  await expect(page.locator("#speak")).toBeEnabled();
  await expect(page.locator("#status")).toContainText("Ответ прерван");
  expect(voiceRequests).toBe(1);

  // STOP without a receipt is not a successful close. Keep retry available,
  // but do not allow a fresh microphone turn on the uncertain remote session.
  await page.unroute("**/api/rt1/visitor/close");
  await page.route("**/api/rt1/visitor/close", (route) =>
    route.fulfill({ status: 502, contentType: "application/json", body: JSON.stringify({ code: "RT1_REMOTE_STOP_UNCONFIRMED" }) }));
  await page.locator("#leave").click();
  await expect(page.locator("#leave")).toBeEnabled();
  await expect(page.locator("#speak")).toBeDisabled();
  await expect(page.locator("#status")).toContainText("RT1_REMOTE_STOP_UNCONFIRMED");
  expect(voiceRequests).toBe(1);
});
