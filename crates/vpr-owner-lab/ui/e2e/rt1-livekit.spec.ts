import { expect, test } from "@playwright/test";

test("RT1 uses existing LiveKit SDK, attaches provider media and closes on revoked publication", async ({ page }) => {
  await page.addInitScript(() => {
    const calls: string[] = [];
    const win = window as typeof window & { __rt1LivekitCalls?: string[]; LivekitClient?: unknown };
    win.__rt1LivekitCalls = calls;
    class FakeRoom {
      canPlaybackAudio = true;
      listeners = new Map<string, (...args: unknown[]) => void>();
      on(event: string, callback: (...args: unknown[]) => void): FakeRoom {
        this.listeners.set(event, callback);
        return this;
      }
      async connect(server: string, token: string): Promise<void> {
        calls.push("connect:" + server + ":" + token);
        const track = (kind: string) => ({
          kind,
          attach: () => calls.push("attach:" + kind),
          detach: () => calls.push("detach:" + kind),
        });
        this.listeners.get("subscribed")?.(track("video"));
        this.listeners.get("subscribed")?.(track("audio"));
      }
      async startAudio(): Promise<void> { calls.push("startAudio"); }
      async disconnect(): Promise<void> {
        calls.push("disconnect");
        this.listeners.get("disconnected")?.();
      }
    }
    win.LivekitClient = {
      Room: FakeRoom,
      RoomEvent: {
        TrackSubscribed: "subscribed",
        TrackUnsubscribed: "unsubscribed",
        Disconnected: "disconnected",
      },
    };
  });

  let active = true;
  let closed = 0;
  await page.route("**/api/bootstrap", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ csrf_token: "test" }) }));
  await page.route("**/api/rt1/visitor/start", (route) =>
    route.fulfill({
      status: 200, contentType: "application/json",
      body: JSON.stringify({
        session_id: "livekit-visitor",
        signal: { transport: { kind: "live_kit", server_url: "wss://example.test", token: "scoped" } },
      }),
    }));
  await page.route("**/api/rt1/visitor/status", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ active }) }));
  await page.route("**/api/rt1/visitor/close", (route) => {
    closed += 1;
    return route.fulfill({
      status: 200, contentType: "application/json",
      body: JSON.stringify({ ok: true, remote_stop_confirmed: true }),
    });
  });

  await page.goto("/rt1");
  await page.locator("#visitor-token").fill("visitor-invite");
  await page.locator("#consent").check();
  await page.locator("#join").click();
  await expect(page.locator("#status")).toContainText("LiveKit подключён");
  await expect.poll(() => page.evaluate(() => (
    (window as typeof window & { __rt1LivekitCalls?: string[] }).__rt1LivekitCalls ?? []
  ))).toEqual([
    "connect:wss://example.test:scoped",
    "attach:video",
    "attach:audio",
  ]);

  active = false;
  await expect(page.locator("#leave")).toBeDisabled({ timeout: 10_000 });
  await expect.poll(() => closed).toBe(1);
  await expect(page.locator("#status")).toContainText("остановка аудио и видео подтверждена");
  const calls = await page.evaluate(() => (
    (window as typeof window & { __rt1LivekitCalls?: string[] }).__rt1LivekitCalls ?? []
  ));
  expect(calls).toContain("disconnect");
});

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


test("visitor microphone cannot start a second turn while canonical voice request is pending", async ({ page }) => {
  await page.addInitScript(() => {
    const win = window as typeof window & { LivekitClient?: unknown; __fakeProcessor?: { port: { onmessage: ((event: {data: Float32Array}) => void) | null } } };
    class FakeRoom {
      on(): FakeRoom { return this; }
      async connect(): Promise<void> {}
      async disconnect(): Promise<void> {}
    }
    win.LivekitClient = { Room: FakeRoom, RoomEvent: {} };
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
  (release as (() => void) | null)?.();
  await expect(page.locator("#speak")).toBeEnabled();
  expect(voiceRequests).toBe(1);
});
