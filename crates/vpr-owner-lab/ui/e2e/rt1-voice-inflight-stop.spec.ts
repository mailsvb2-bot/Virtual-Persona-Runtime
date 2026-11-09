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


test("late WebRTC offer resolution after pagehide never submits SDP or ICE", async ({ page }) => {
  await page.addInitScript(() => {
    const win = window as typeof window & { __completeRemote?: () => void };
    class DeferredPeer {
      localDescription = { type: "answer", sdp: "local-answer" };
      iceGatheringState = "complete";
      connectionState = "new";
      onicecandidate: ((event: unknown) => void) | null = null;
      ontrack: ((event: unknown) => void) | null = null;
      onconnectionstatechange: (() => void) | null = null;
      async setRemoteDescription(): Promise<void> {
        await new Promise<void>((resolve) => { win.__completeRemote = resolve; });
      }
      async createAnswer(): Promise<{ type: string; sdp: string }> {
        return { type: "answer", sdp: "local-answer" };
      }
      async setLocalDescription(): Promise<void> {}
      close(): void { this.connectionState = "closed"; }
    }
    (win as unknown as { RTCPeerConnection: unknown }).RTCPeerConnection = DeferredPeer;
    Object.defineProperty(HTMLMediaElement.prototype, "play", {
      configurable: true, value: async () => {},
    });
  });
  await page.route("**/api/bootstrap", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ csrf_token: "test" }) }));
  await page.route("**/api/rt1/visitor/start", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({
      session_id: "revoked-before-answer", voice_enabled: true, diagnostic_rtp: true,
      signal: { transport: { kind: "web_rtc", offer: { type: "offer", sdp: "remote-offer" }, ice_servers: [] } },
    }) }));
  let answers = 0;
  let ice = 0;
  await page.route("**/api/rt1/visitor/answer", route => {
    answers += 1;
    return route.fulfill({ status: 200, contentType: "application/json", body: "{}" });
  });
  await page.route("**/api/rt1/visitor/ice", route => {
    ice += 1;
    return route.fulfill({ status: 200, contentType: "application/json", body: "{}" });
  });
  await page.route("**/api/rt1/visitor/close", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ remote_stop_confirmed: true }) }));
  await page.goto("/rt1");
  await page.locator("#visitor-token").fill("scoped-invite");
  await page.locator("#consent").check();
  await page.locator("#join").click();
  await expect.poll(() => page.evaluate(() =>
    typeof (window as typeof window & { __completeRemote?: () => void }).__completeRemote
  )).toBe("function");
  // START alone must not expose microphone capture before SDP acceptance.
  await expect(page.locator("#speak")).toBeDisabled();
  await page.evaluate(() => window.dispatchEvent(new Event("pagehide")));
  await page.evaluate(() =>
    (window as typeof window & { __completeRemote?: () => void }).__completeRemote?.()
  );
  await expect(page.locator("#status")).toContainText("отозвана");
  expect(answers).toBe(0);
  expect(ice).toBe(0);
});
