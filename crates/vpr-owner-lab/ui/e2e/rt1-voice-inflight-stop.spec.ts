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


test("pagehide while START is pending attempts late sender STOP without connecting media", async ({ page }) => {
  await page.addInitScript(() => {
    const win = window as typeof window & { __peerConstructed?: boolean };
    win.__peerConstructed = false;
    class FakePeer {
      iceGatheringState = "complete";
      connectionState = "connected";
      localDescription = { type: "answer", sdp: "valid-fake-answer" };
      onicecandidate: ((event: unknown) => void) | null = null;
      ontrack: ((event: unknown) => void) | null = null;
      onconnectionstatechange: (() => void) | null = null;
      constructor() { win.__peerConstructed = true; }
      async setRemoteDescription(): Promise<void> {}
      async createAnswer(): Promise<{ type: string; sdp: string }> {
        return { type: "answer", sdp: "valid-fake-answer" };
      }
      async setLocalDescription(): Promise<void> {}
      close(): void { this.connectionState = "closed"; }
    }
    (win as unknown as { RTCPeerConnection: unknown }).RTCPeerConnection = FakePeer;
  });
  await page.route("**/api/bootstrap", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ csrf_token: "test" }) }));
  let releaseStart: (() => void) | null = null;
  await page.route("**/api/rt1/visitor/start", async route => {
    await new Promise<void>(resolve => { releaseStart = resolve; });
    await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({
      session_id: "created-after-pagehide", voice_enabled: true,
      signal: { transport: { kind: "web_rtc", offer: { type: "offer", sdp: "remote-offer" } } },
    }) });
  });
  let closes = 0;
  let answers = 0;
  await page.route("**/api/rt1/visitor/close", route => {
    closes += 1;
    return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({
      ok: true, remote_stop_confirmed: true,
    }) });
  });
  await page.route("**/api/rt1/visitor/answer", route => {
    answers += 1;
    return route.fulfill({ status: 200, contentType: "application/json", body: "{}" });
  });
  await page.goto("/rt1");
  await page.locator("#visitor-token").fill("scoped-invite");
  await page.locator("#consent").check();
  await page.locator("#join").click();
  await expect.poll(() => Boolean(releaseStart)).toBe(true);
  // There is no session ID yet. A best-effort close is only possible after
  // the pending START eventually returns its exact operation identity.
  await page.evaluate(() => window.dispatchEvent(new Event("pagehide")));
  (releaseStart as (() => void) | null)?.();
  await expect.poll(() => closes).toBe(1);
  // A healthy peer mock would connect without the pagehide fence. This test
  // must fail if startup constructs a WebRTC peer after the tab has left.
  expect(await page.evaluate(() =>
    (window as typeof window & { __peerConstructed?: boolean }).__peerConstructed
  )).toBe(false);
  expect(answers).toBe(0);
  await expect(page.locator("#speak")).toBeDisabled();
});


test("RT1 text validation and late-response fence survive owner revoke", async ({ page }) => {
  await page.addInitScript(() => {
    const win = window as typeof window & { LivekitClient?: unknown };
    class FakeRoom {
      on(): FakeRoom { return this; }
      async connect(): Promise<void> {}
      async disconnect(): Promise<void> {}
    }
    win.LivekitClient = { Room: FakeRoom, RoomEvent: {} };
  });
  await page.route("**/api/bootstrap", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ csrf_token: "test" }) }));
  await page.route("**/api/rt1/visitor/start", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({
      session_id: "text-revoke", voice_enabled: true,
      signal: { transport: { kind: "live_kit", server_url: "wss://example.test", token: "scoped" } },
    }) }));
  await page.route("**/api/rt1/visitor/status", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ active: true }) }));
  await page.route("**/api/rt1/visitor/close", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ remote_stop_confirmed: true }) }));
  // Owner controls are driven by the authenticated canonical status route,
  // never enabled merely because a key was typed into the browser.
  await page.route("**/api/rt1/owner/status", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({
      owner_id: "test-owner", persona_id: "test-persona", persona_version: 2,
      publication_status: "published",
    }) }));
  await page.route("**/api/rt1/owner/pause", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ ok: true }) }));
  let releaseReply: (() => void) | null = null;
  let textRequests = 0;
  await page.route("**/api/rt1/visitor/text", async route => {
    textRequests += 1;
    await new Promise<void>(resolve => { releaseReply = resolve; });
    await route.fulfill({
      status: 200, contentType: "application/json",
      body: JSON.stringify({ reply: "This stale reply must never render" }),
    }).catch(() => {}); // The browser intentionally aborts this request.
  });

  const pageErrors: string[] = [];
  page.on("pageerror", error => pageErrors.push(error.message));
  await page.goto("/rt1");
  await page.locator("#visitor-token").fill("invite");
  await page.locator("#consent").check();
  await page.locator("#join").click();
  await expect(page.locator("#send-text")).toBeEnabled();

  await page.locator("#send-text").click();
  await expect(page.locator("#status")).toContainText("Введите вопрос");
  expect(textRequests).toBe(0);
  // The HTML maxlength prevents ordinary typing beyond 8192. Bypass only
  // that DOM attribute to exercise the independent JavaScript validation
  // against a programmatically populated input.
  await page.locator("#visitor-text").evaluate((input) => {
    const field = input as HTMLInputElement;
    field.removeAttribute("maxlength");
    field.value = "x".repeat(8193);
  });
  await page.locator("#send-text").click();
  await expect(page.locator("#status")).toContainText("до 8192 символов");
  expect(textRequests).toBe(0);
  expect(pageErrors).toEqual([]);

  await page.locator("#visitor-text").fill("Что нового?");
  await page.locator("#send-text").click();
  await expect.poll(() => textRequests).toBe(1);
  await expect(page.locator("#send-text")).toBeDisabled();
  await page.locator("#owner-token").fill("owner-key");
  await page.locator("#owner-refresh").click();
  await expect(page.locator("#pause")).toBeEnabled();
  await page.locator("#pause").click();
  await expect(page.locator("#status")).toContainText("Публикация отозвана");
  (releaseReply as (() => void) | null)?.();
  await expect(page.locator("#status")).toContainText("Публикация отозвана");
  await expect(page.locator("#speech-result")).not.toContainText("stale reply");
  expect(pageErrors).toEqual([]);
});


test("owner actions require canonical authenticated status and current server PersonaVersion", async ({ page }) => {
  await page.route("**/api/bootstrap", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ csrf_token: "test" }) }));
  await page.route("**/api/rt1/owner/status", route => {
    const token = JSON.parse(route.request().postData() ?? "{}").owner_token;
    if (token !== "valid-owner") {
      return route.fulfill({ status: 403, contentType: "application/json", body: JSON.stringify({ code: "RT1_OWNER_STATUS_DENIED" }) });
    }
    return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({
      owner_id: "owner-1", persona_id: "owner-persona", persona_version: 7,
      publication_status: "published",
    }) });
  });
  await page.goto("/rt1");
  await expect(page.locator("#status")).toHaveAttribute("data-rt1-ready", "true");
  await expect(page.locator("#publish")).toBeDisabled();
  await expect(page.locator("#pause")).toBeDisabled();
  await expect(page.locator("#unpublish")).toBeDisabled();
  await expect(page.locator("#persona-version")).toHaveAttribute("readonly", "");

  await page.locator("#owner-token").fill("invalid-owner");
  await page.locator("#owner-refresh").click();
  await expect(page.locator("#status")).toContainText("RT1_OWNER_STATUS_DENIED");
  await expect(page.locator("#publish")).toBeDisabled();
  await expect(page.locator("#pause")).toBeDisabled();

  await page.locator("#owner-token").fill("valid-owner");
  await page.locator("#owner-refresh").click();
  await expect(page.locator("#persona-version")).toHaveValue("7");
  await expect(page.locator("#owner-state")).toContainText("опубликован");
  await expect(page.locator("#pause")).toBeEnabled();
  await expect(page.locator("#unpublish")).toBeEnabled();
  await expect(page.locator("#publish")).toBeDisabled();

  // Editing the credential immediately discards the prior owner projection.
  await page.locator("#owner-token").fill("different-owner");
  await expect(page.locator("#persona-version")).toHaveValue("");
  await expect(page.locator("#pause")).toBeDisabled();
  await expect(page.locator("#unpublish")).toBeDisabled();
});


test("RT1 local owner invitation reaches separate visitor screen without leaking owner controls or auto-joining", async ({ page, context }) => {
  const invitation = "A".repeat(64);
  let publication = false;
  let starts = 0;
  await page.route("**/api/bootstrap", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ csrf_token: "test" }) }));
  await page.route("**/api/rt1/owner/status", route =>
    route.fulfill({
      status: 200, contentType: "application/json",
      body: JSON.stringify({
        owner_id: "owner-local", persona_id: "persona-reviewed", persona_version: 8,
        publication_status: publication ? "published" : null,
      }),
    }));
  await page.route("**/api/rt1/owner/publish", route => {
    const body = JSON.parse(route.request().postData() || "{}");
    expect(body.expected_persona_version).toBe(8);
    expect(body.owner_token).toBe("local-owner-key");
    publication = true;
    return route.fulfill({
      status: 200, contentType: "application/json",
      body: JSON.stringify({ visitor_token: invitation }),
    });
  });
  await page.goto("/rt1");
  await page.locator("#owner-token").fill("local-owner-key");
  await page.locator("#owner-refresh").click();
  await expect(page.locator("#publish")).toBeEnabled();
  await page.locator("#publish").click();
  await expect(page.locator("#invite-link")).toHaveValue(
    new RegExp("/rt1/visit#invite=" + invitation + "$"),
  );
  const link = await page.locator("#invite-link").inputValue();
  expect(link).toContain("/rt1/visit#invite=");
  expect(link).not.toContain("local-owner-key");

  const visitor = await context.newPage();
  await visitor.route("**/api/bootstrap", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ csrf_token: "test" }) }));
  await visitor.route("**/api/rt1/visitor/start", route => {
    starts += 1;
    return route.fulfill({
      status: 403, contentType: "application/json",
      body: JSON.stringify({ code: "RT1_VISITOR_START_DENIED" }),
    });
  });
  await visitor.goto(link);
  await expect(visitor.locator("#status")).toHaveAttribute("data-rt1-ready", "true");
  await expect(visitor.locator("#owner-panel")).toBeHidden();
  await expect(visitor.locator("#visitor-panel")).toBeVisible();
  await expect(visitor.locator("#visitor-token")).toHaveValue(invitation);
  expect(new URL(visitor.url()).hash).toBe("");
  await expect(visitor.locator("#consent")).not.toBeChecked();
  expect(starts).toBe(0);
  await visitor.locator("#join").click();
  await expect(visitor.locator("#status")).toContainText("согласие");
  expect(starts).toBe(0);
  await visitor.locator("#consent").check();
  await visitor.locator("#join").click();
  await expect(visitor.locator("#status")).toContainText("RT1_VISITOR_START_DENIED");
  expect(starts).toBe(1);
  await expect(visitor.locator("#speak")).toBeDisabled();
  await visitor.close();
});

test("RT1 invitation is immediately invalidated on owner credential change", async ({ page }) => {
  await page.route("**/api/bootstrap", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ csrf_token: "test" }) }));
  await page.route("**/api/rt1/owner/status", route =>
    route.fulfill({
      status: 200, contentType: "application/json",
      body: JSON.stringify({
        owner_id: "owner-1", persona_id: "persona", persona_version: 2,
        publication_status: "published",
      }),
    }));
  await page.goto("/rt1");
  await page.locator("#owner-token").fill("owner-key");
  await page.locator("#owner-refresh").click();
  await expect(page.locator("#copy-invite")).toBeDisabled();
  // An invitation must only originate from actual publish authority; simply
  // supplying a valid owner token/status does not reconstruct a bearer token.
  await expect(page.locator("#invite-link")).toHaveValue("");
  await page.locator("#owner-token").fill("another-owner");
  await expect(page.locator("#copy-invite")).toBeDisabled();
  await expect(page.locator("#invite-token")).toHaveValue("");
  await expect(page.locator("#invite-link")).toHaveValue("");
});


test("RT1 text failures display only reviewed actionable reason codes", async ({ page }) => {
  await page.addInitScript(() => {
    const win = window as typeof window & { LivekitClient?: unknown };
    class FakeRoom {
      on(): FakeRoom { return this; }
      async connect(): Promise<void> {}
      async disconnect(): Promise<void> {}
    }
    win.LivekitClient = { Room: FakeRoom, RoomEvent: {} };
  });
  await page.route("**/api/bootstrap", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ csrf_token: "test" }) }));
  await page.route("**/api/rt1/visitor/start", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({
      session_id: "typed-error-test", voice_enabled: true,
      signal: { transport: { kind: "live_kit", server_url: "wss://example.test", token: "scoped" } },
    }) }));
  await page.route("**/api/rt1/visitor/status", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ active: true }) }));
  await page.route("**/api/rt1/visitor/close", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ remote_stop_confirmed: true }) }));
  let code = "PROVIDER_RATE_LIMITED";
  await page.route("**/api/rt1/visitor/text", route =>
    route.fulfill({ status: code === "PROVIDER_RATE_LIMITED" ? 429 : 409,
      contentType: "application/json", body: JSON.stringify({ code }) }));
  await page.goto("/rt1");
  await page.locator("#visitor-token").fill("invite");
  await page.locator("#consent").check();
  await page.locator("#join").click();
  await expect(page.locator("#send-text")).toBeEnabled();
  await page.locator("#visitor-text").fill("Первый вопрос");
  await page.locator("#send-text").click();
  await expect(page.locator("#status")).toContainText("Достигнут лимит запросов провайдера");
  await expect(page.locator("#status")).toContainText("PROVIDER_RATE_LIMITED");

  code = "RT1_VISITOR_TEXT_DENIED";
  await page.locator("#visitor-text").fill("Второй вопрос");
  await page.locator("#send-text").click();
  await expect(page.locator("#status")).toContainText("RT1_VISITOR_TEXT_DENIED");
  await expect(page.locator("#status")).not.toContainText("лимит запросов");
});

for (const [actionId, privacyRoute] of [
  ["memory-disable", "consent"],
  ["memory-erase", "erase"],
  ["memory-revoke", "revoke"],
  ["memory-rotate", "rotate"],
] as const) {
  test(`RT1 ${actionId} preserves STOP retry when privacy endpoint cannot confirm media stop`, async ({ page }) => {
    await page.addInitScript(() => {
      const win = window as typeof window & { LivekitClient?: unknown };
      class FakeRoom {
        on(): FakeRoom { return this; }
        async connect(): Promise<void> {}
        async disconnect(): Promise<void> {}
      }
      win.LivekitClient = { Room: FakeRoom, RoomEvent: {} };
      window.confirm = () => true;
    });
    await page.route("**/api/bootstrap", route => route.fulfill({
      status: 200, contentType: "application/json",
      body: JSON.stringify({ csrf_token: "test", rt1_memory_configured: true }),
    }));
    await page.route("**/api/rt1/visitor/start", route => route.fulfill({
      status: 200, contentType: "application/json",
      body: JSON.stringify({
        session_id: "privacy-stop-session", voice_enabled: true,
        signal: { transport: { kind: "live_kit", server_url: "wss://example.test", token: "scoped" } },
      }),
    }));
    await page.route("**/api/rt1/visitor/status", route => route.fulfill({
      status: 200, contentType: "application/json", body: JSON.stringify({ active: true }),
    }));
    let privacyCalls = 0;
    await page.route(`**/api/rt1/visitor/memory/${privacyRoute}`, route => {
      privacyCalls += 1;
      return route.fulfill({
        status: 502, contentType: "application/json",
        body: JSON.stringify({ code: "RT1_MEMORY_REMOTE_STOP_UNCONFIRMED" }),
      });
    });
    let closeCalls = 0;
    await page.route("**/api/rt1/visitor/close", route => {
      closeCalls += 1;
      expect(route.request().postDataJSON()).toEqual({
        session_id: "privacy-stop-session",
      });
      return route.fulfill({
        status: 200, contentType: "application/json",
        body: JSON.stringify({ ok: true, remote_stop_confirmed: true }),
      });
    });
    await page.goto("/rt1");
    await page.locator("#visitor-token").fill("invite");
    await page.locator("#consent").check();
    await page.locator("#join").click();
    await expect(page.locator("#leave")).toBeEnabled();
    await expect(page.locator("#memory-panel")).toBeVisible();
    await page.locator("#memory-token").fill("a".repeat(64));
    await page.locator("#" + actionId).click();
    await expect.poll(() => privacyCalls).toBe(1);
    await expect(page.locator("#status")).toContainText("RT1_MEMORY_REMOTE_STOP_UNCONFIRMED");
    await expect(page.locator("#leave")).toBeEnabled();
    await expect(page.locator("#join")).toBeDisabled();
    await expect(page.locator("#speak")).toBeDisabled();
    await expect(page.locator("#send-text")).toBeDisabled();
    await page.locator("#leave").click();
    await expect.poll(() => closeCalls).toBe(1);
    await expect(page.locator("#leave")).toBeDisabled();
    await expect(page.locator("#join")).toBeEnabled();
  });
}
