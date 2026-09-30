import { expect, test, type Page, type Route } from "@playwright/test";

const csrfToken = "e2e-csrf-token";
const questions = [
  { claim_id: "opinion-working-style", prompt: "Как вы предпочитаете работать?", kind: "opinion" },
  { claim_id: "value-priority", prompt: "Что для вас важно?", kind: "value_judgment" },
  { claim_id: "preference-tone", prompt: "Какой стиль общения вы предпочитаете?", kind: "preference" },
];

type Claim = {
  claim_id: string;
  statement: string;
  kind: string;
  verification: string;
  revision: number;
  owner_reviewed: boolean;
};

type FixtureState = {
  personaId: string;
  personaVersion: number;
  captureState: null | "draft" | "captured" | "reviewed";
  answers: string[];
  claims: Claim[];
  ownerReviewed: boolean;
  sessionState: string;
  sessionAudience: null | "owner" | "visitor";
  avatarOpen: boolean;
  voiceReady: boolean;
  videoReady: boolean;
  transportKind: "web_rtc" | "live_kit";
  startAudiences: string[];
  directSpeech: string[];
  textMessages: string[];
  apiPaths: string[];
  staleStatusOnceAfterStart: boolean;
  voiceReference: null | { bytes: number; media_type: string; sha256: string; raw_retained: false };
  appearanceReference: null | { bytes: number; media_type: string; sha256: string; raw_retained: false };
};

const installBrowserFakes = async (page: Page): Promise<void> => {
  await page.addInitScript(() => {
    class FakeAudioContext {
      sampleRate: number;
      constructor(options?: AudioContextOptions) {
        this.sampleRate = options?.sampleRate ?? 48_000;
      }
      audioWorklet = { addModule: async () => undefined };
      async resume(): Promise<void> {}
      async close(): Promise<void> {}
    }

    class FakePeerConnection {
      connectionState = "new";
      ontrack: ((event: unknown) => void) | null = null;
      onconnectionstatechange: (() => void) | null = null;
      onicecandidate: ((event: unknown) => void) | null = null;
      async setRemoteDescription(): Promise<void> {}
      async createAnswer(): Promise<{ type: "answer"; sdp: string }> {
        return { type: "answer", sdp: "v=0" };
      }
      async setLocalDescription(): Promise<void> {
        this.connectionState = "connected";
        queueMicrotask(() => this.onconnectionstatechange?.());
      }
      close(): void {
        this.connectionState = "closed";
        this.onconnectionstatechange?.();
      }
    }

    Object.defineProperty(window, "AudioContext", { value: FakeAudioContext });
    Object.defineProperty(window, "RTCPeerConnection", { value: FakePeerConnection });
  });
};

const installLiveKitBrowserFake = async (page: Page): Promise<void> => {
  await page.addInitScript(() => {
    type EventHandler = (...args: unknown[]) => void;
    const roomEvents = {
      TrackSubscribed: "track-subscribed",
      TrackUnsubscribed: "track-unsubscribed",
      DataReceived: "data-received",
      Reconnecting: "reconnecting",
      Reconnected: "reconnected",
      Disconnected: "disconnected",
    };

    class FakeRemoteTrack {
      readonly mediaStreamTrack = undefined;
      constructor(readonly kind: "audio" | "video") {}
      attach(element: HTMLMediaElement): HTMLMediaElement {
        element.style.width = "4096px";
        element.style.height = "4096px";
        return element;
      }
      async getRTCStatsReport(): Promise<RTCStatsReport> {
        const timestamp = this.kind === "audio" ? 1_000 : 1_035;
        const report = new Map<string, unknown>([
          [
            `${this.kind}-inbound`,
            {
              type: "inbound-rtp",
              kind: this.kind,
              packetsReceived: 12,
              estimatedPlayoutTimestamp: timestamp,
            },
          ],
        ]);
        return report as unknown as RTCStatsReport;
      }
    }

    class FakeRoom {
      readonly localParticipant = {
        sendText: async (): Promise<void> => undefined,
      };
      private readonly handlers = new Map<string, EventHandler[]>();

      on(event: string, handler: EventHandler): FakeRoom {
        const handlers = this.handlers.get(event) ?? [];
        handlers.push(handler);
        this.handlers.set(event, handlers);
        return this;
      }

      private emit(event: string, ...args: unknown[]): void {
        for (const handler of this.handlers.get(event) ?? []) handler(...args);
      }

      async connect(): Promise<void> {
        this.emit(roomEvents.TrackSubscribed, new FakeRemoteTrack("video"));
        this.emit(roomEvents.TrackSubscribed, new FakeRemoteTrack("audio"));
      }

      async disconnect(): Promise<void> {}

      triggerUnexpectedDisconnect(): void {
        this.emit(roomEvents.Disconnected);
      }
    }

    const fakeWindow = window as typeof window & {
      LivekitClient?: unknown;
      __vprFakeLiveKitDisconnect?: () => void;
    };
    fakeWindow.LivekitClient = {
      Room: class extends FakeRoom {
        constructor() {
          super();
          fakeWindow.__vprFakeLiveKitDisconnect = () => this.triggerUnexpectedDisconnect();
        }
      },
      RoomEvent: roomEvents,
    };

    Object.defineProperty(HTMLVideoElement.prototype, "requestVideoFrameCallback", {
      configurable: true,
      value(callback: () => void): number {
        queueMicrotask(callback);
        return 1;
      },
    });
  });
};

const json = async (route: Route, payload: unknown, status = 200): Promise<void> => {
  await route.fulfill({ status, contentType: "application/json", body: JSON.stringify(payload) });
};

const captureSnapshot = (state: FixtureState) => ({
  persona_id: state.personaId,
  persona_version: state.personaVersion,
  capture_state: state.captureState,
  answered_count: state.answers.length,
  question_count: questions.length,
  current_question: state.captureState === "draft" ? questions[state.answers.length] ?? null : null,
  claims: state.claims,
});

const statusSnapshot = (state: FixtureState) => ({
  session_state: state.sessionState,
  avatar_open: state.avatarOpen,
  egress_enabled: true,
  conversation_readiness: "text_and_voice",
  modality_readiness: state.ownerReviewed
    ? {
        text: "ready",
        voice: state.voiceReady ? "ready" : state.sessionState === "active" ? "preparing" : "not_ready",
        video: state.videoReady ? "ready" : state.sessionState === "active" ? "preparing" : "not_ready",
      }
    : { text: "not_ready", voice: "not_ready", video: "not_ready" },
  session_audience: state.sessionAudience,
  owner_context_state: state.ownerReviewed ? "reviewed" : "missing",
  persona_version: state.personaVersion,
  reviewed_owner_claims: state.sessionAudience === "visitor"
    ? 0
    : state.claims.filter((claim) => claim.owner_reviewed).length,
});

const reviewedProfile = (state: FixtureState) => ({
  persona_id: state.personaId,
  persona_version: state.personaVersion,
  claims: state.claims.map(({ claim_id, statement, kind, revision }) => ({
    claim_id, statement, kind, revision,
  })),
});

const installApiFixture = async (page: Page, state: FixtureState): Promise<void> => {
  await page.route("**/api/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    state.apiPaths.push(`${request.method()} ${path}`);

    if (request.method() === "POST") {
      expect(request.headers()["x-vpr-csrf"]).toBe(csrfToken);
    }

    if (request.method() === "GET" && path === "/api/bootstrap") {
      return json(route, { csrf_token: csrfToken, egress_enabled: true });
    }
    if (request.method() === "GET" && path === "/api/status") {
      if (state.staleStatusOnceAfterStart && state.startAudiences.length > 0) {
        state.staleStatusOnceAfterStart = false;
        return json(route, {
          ...statusSnapshot(state),
          session_state: "none",
          avatar_open: false,
          session_audience: null,
        });
      }
      return json(route, statusSnapshot(state));
    }
    if (request.method() === "GET" && path === "/api/persona/capture") {
      if (state.captureState === null || state.ownerReviewed) {
        return json(route, {
          capture: null,
          owner_context_state: state.ownerReviewed ? "reviewed" : "missing",
          persona_version: state.personaVersion,
          reviewed_owner_claims: state.claims.filter((claim) => claim.owner_reviewed).length,
        });
      }
      return json(route, captureSnapshot(state));
    }
    if (request.method() === "GET" && path === "/api/evidence/session") {
      return json(route, { session_state: state.sessionState, fixture: true });
    }
    if (request.method() === "GET" && path === "/api/references") {
      return json(route, {
        voice: state.voiceReference,
        appearance: state.appearanceReference,
      });
    }
    if (request.method() === "POST" && path === "/api/evidence/session/export") {
      const participantRole = state.startAudiences.at(-1) ?? "owner";
      return json(route, {
        session_sequence: Math.max(state.startAudiences.length, 1),
        participant_role: participantRole,
      });
    }

    if (request.method() === "POST" && path === "/api/references/intake") {
      expect(request.headers()["x-vpr-reference-consent"]).toBe("confirmed");
      const kind = request.headers()["x-vpr-reference-kind"];
      const mediaType = request.headers()["x-vpr-reference-media-type"];
      const bytes = request.postDataBuffer()?.byteLength ?? 0;
      const metadata = {
        bytes,
        media_type: mediaType,
        sha256: kind === "voice" ? "a".repeat(64) : "b".repeat(64),
        raw_retained: false as const,
      };
      if (kind === "voice") state.voiceReference = metadata;
      else if (kind === "appearance") state.appearanceReference = metadata;
      else return json(route, { ok: false, code: "INVALID_INPUT" }, 400);
      return json(route, { kind, ...metadata }, 201);
    }

    const body = request.postDataJSON() as Record<string, unknown>;
    if (path === "/api/references/clear") {
      const kind = String(body.kind ?? "");
      if (kind === "voice") state.voiceReference = null;
      else if (kind === "appearance") state.appearanceReference = null;
      else return json(route, { ok: false, code: "INVALID_INPUT" }, 400);
      return json(route, { ok: true, kind, cleared: true });
    }
    if (path === "/api/persona/create") {
      state.personaId = String(body.persona_id);
      state.personaVersion = 1;
      state.captureState = "draft";
      state.answers = [];
      state.claims = [];
      return json(route, captureSnapshot(state));
    }
    if (path === "/api/persona/capture/answer") {
      const question = questions[state.answers.length];
      if (!question) return json(route, { ok: false, code: "INVALID_STATE_TRANSITION" }, 409);
      const statement = String(body.answer);
      state.answers.push(statement);
      state.claims.push({
        claim_id: question.claim_id,
        statement,
        kind: question.kind,
        verification: "unverified",
        revision: 1,
        owner_reviewed: false,
      });
      return json(route, captureSnapshot(state));
    }
    if (path === "/api/persona/capture/finish") {
      state.captureState = "captured";
      return json(route, captureSnapshot(state));
    }
    if (path === "/api/persona/claims/approve") {
      const claim = state.claims.find((candidate) => candidate.claim_id === String(body.claim_id));
      if (!claim) return json(route, { ok: false, code: "INVALID_INPUT" }, 400);
      claim.owner_reviewed = true;
      claim.verification = "verified";
      return json(route, captureSnapshot(state));
    }
    if (path === "/api/persona/claims/correct") {
      const claim = state.claims.find((candidate) => candidate.claim_id === String(body.claim_id));
      if (!claim) return json(route, { ok: false, code: "INVALID_INPUT" }, 400);
      claim.statement = String(body.statement);
      claim.kind = String(body.kind);
      claim.owner_reviewed = true;
      claim.verification = "verified";
      claim.revision += 1;
      if (state.ownerReviewed) {
        state.personaVersion += 1;
        state.voiceReady = false;
        state.videoReady = false;
      }
      if (state.ownerReviewed) {
        return json(route, {
          owner_context_state: "reviewed",
          persona_version: state.personaVersion,
          reviewed_owner_claims: state.claims.length,
        });
      }
      return json(route, captureSnapshot(state));
    }
    if (path === "/api/persona/review/complete") {
      state.captureState = "reviewed";
      state.ownerReviewed = true;
      state.personaVersion = 2;
      return json(route, {
        owner_context_state: "reviewed",
        persona_version: state.personaVersion,
        reviewed_owner_claims: state.claims.length,
      });
    }
    if (path === "/api/persona/reviewed") {
      if (state.sessionAudience === "visitor") {
        return json(route, { ok: false, code: "AUTH_SCOPE_DENIED" }, 403);
      }
      return json(route, reviewedProfile(state));
    }
    if (path === "/api/evidence/media") {
      const kind = String(body.kind ?? "");
      if (kind === "audio_started") state.voiceReady = true;
      if (kind === "video_ready") state.videoReady = true;
      return json(route, { ok: true });
    }
    if (path === "/api/avatar/start") {
      const audience = String(body.audience ?? "owner");
      state.startAudiences.push(audience);
      state.sessionAudience = audience as "owner" | "visitor";
      state.sessionState = "active";
      state.avatarOpen = true;
      const transport = state.transportKind === "live_kit"
        ? { kind: "live_kit", server_url: "wss://livekit.example.test", token: "fixture-token" }
        : {
            kind: "web_rtc",
            offer: { kind: "offer", sdp: "v=0" },
            ice_servers: [],
          };
      return json(route, {
        evidence_session_sequence: state.startAudiences.length,
        transport,
        capabilities: ["text", "interrupt"],
        client_control: state.transportKind === "live_kit"
          ? {
              event_route: null,
              interrupt: true,
              interrupt_requires_playback_id: false,
              text_input: true,
            }
          : null,
      });
    }
    if (path === "/api/avatar/answer" || path === "/api/avatar/ice") {
      return json(route, { ok: true });
    }
    if (path === "/api/text/turn") {
      expect(Number(request.headers()["x-vpr-evidence-request"])).toBeGreaterThan(0);
      const text = String(body.text);
      state.textMessages.push(`${state.sessionAudience}:${text}`);
      return json(route, {
        reply: state.sessionAudience === "visitor"
          ? "Visitor scoped text reply"
          : "Owner scoped text reply",
        locale: "ru-RU",
        evidence_turn_sequence: state.textMessages.length,
        evidence_output_sequence: state.textMessages.length,
        first_meaningful_response_millis: 100,
        total_millis: 150,
      });
    }
    if (path === "/api/avatar/speak") {
      if (state.sessionAudience === "visitor") {
        return json(route, { ok: false, code: "AUTH_SCOPE_DENIED" }, 403);
      }
      state.directSpeech.push(String(body.text));
      return json(route, { ok: true });
    }
    if (path === "/api/session/revoke") {
      state.sessionState = "revoked";
      state.avatarOpen = false;
      return json(route, { ok: true });
    }
    if (path === "/api/session/close") {
      state.sessionState = "closed";
      state.sessionAudience = null;
      state.avatarOpen = false;
      return json(route, { ok: true });
    }
    return json(route, { ok: false, code: "NOT_FOUND" }, 404);
  });
};

const initialState = (): FixtureState => ({
  personaId: "",
  personaVersion: 1,
  captureState: null,
  answers: [],
  claims: [],
  ownerReviewed: false,
  sessionState: "none",
  sessionAudience: null,
  avatarOpen: false,
  voiceReady: false,
  videoReady: false,
  transportKind: "web_rtc",
  startAudiences: [],
  directSpeech: [],
  textMessages: [],
  apiPaths: [],
  staleStatusOnceAfterStart: false,
  voiceReference: null,
  appearanceReference: null,
});


test("connect closes a started backend session when authoritative start status mismatches", async ({ page }) => {
  const state = initialState();
  state.personaId = "owner-authoritative-start-e2e";
  state.personaVersion = 2;
  state.captureState = "reviewed";
  state.ownerReviewed = true;
  state.staleStatusOnceAfterStart = true;
  state.claims = [{
    claim_id: "opinion-working-style",
    statement: "Отвечай кратко и спокойно",
    kind: "opinion",
    verification: "verified",
    revision: 1,
    owner_reviewed: true,
  }];

  await installBrowserFakes(page);
  await installApiFixture(page, state);
  await page.goto("/");

  await page.locator("#consent").check();
  await page.getByRole("button", { name: "Подключить аватар" }).click();

  await expect(page.locator("#status")).toContainText("SESSION_START_STATE_MISMATCH");
  await expect.poll(() => state.sessionState).toBe("closed");
  expect(state.apiPaths).toContain("POST /api/avatar/start");
  expect(state.apiPaths).toContain("POST /api/session/close");
  expect(state.apiPaths).not.toContain("POST /api/avatar/answer");
});


test("bootstrap does not touch microphone runtime before a realtime session", async ({ page }) => {
  const state = initialState();
  state.personaId = "bootstrap-media-boundary";
  state.personaVersion = 2;
  state.captureState = "reviewed";
  state.ownerReviewed = true;
  state.claims = [{
    claim_id: "opinion-working-style",
    statement: "Отвечай кратко и спокойно",
    kind: "opinion",
    verification: "verified",
    revision: 1,
    owner_reviewed: true,
  }];

  await page.addInitScript(() => {
    const testWindow = window as typeof window & {
      __vprMediaRuntime?: unknown;
      __vprBootstrapMediaAccesses?: number;
    };
    testWindow.__vprBootstrapMediaAccesses = 0;
    const unexpectedMediaAccess = (): never => {
      testWindow.__vprBootstrapMediaAccesses = (testWindow.__vprBootstrapMediaAccesses ?? 0) + 1;
      throw new Error("MEDIA_RUNTIME_TOUCHED_DURING_BOOTSTRAP");
    };
    testWindow.__vprMediaRuntime = {
      mediaDevices: {
        enumerateDevices: async () => unexpectedMediaAccess(),
        getUserMedia: async () => unexpectedMediaAccess(),
        addEventListener: () => unexpectedMediaAccess(),
      },
    };
  });
  await installApiFixture(page, state);
  await page.goto("/");
  await expect(page.locator("#persona-progress")).toContainText("версия 2");
  await expect(page.locator("#connect")).toBeEnabled();
  await expect.poll(() => page.evaluate(
    () => (window as typeof window & { __vprBootstrapMediaAccesses?: number })
      .__vprBootstrapMediaAccesses ?? 0,
  )).toBe(0);
});

test("bootstrap applies one authoritative status snapshot before capture refreshes can resync", async ({ page }) => {
  const state = initialState();
  state.personaId = "bootstrap-reviewed";
  state.personaVersion = 2;
  state.captureState = "reviewed";
  state.ownerReviewed = true;
  state.claims = [{
    claim_id: "opinion-working-style",
    statement: "Отвечай кратко и спокойно",
    kind: "opinion",
    verification: "verified",
    revision: 1,
    owner_reviewed: true,
  }];

  await installBrowserFakes(page);
  await installApiFixture(page, state);
  await page.goto("/");

  await expect(page.locator("#persona-progress")).toContainText("версия 2");
  await expect(page.locator("#status")).toContainText("Persona подтверждена. Готов к подключению");
  await expect(page.locator("#connect")).toBeEnabled();
  expect(state.apiPaths.filter((entry) => entry === "GET /api/status")).toHaveLength(1);
});

test("owner can upload and clear local voice/appearance references without raw retention", async ({ page }) => {
  const state = initialState();
  await installBrowserFakes(page);
  await installApiFixture(page, state);
  await page.goto("/");

  await page.locator("#voice-reference-file").setInputFiles({
    name: "voice.webm",
    mimeType: "audio/webm",
    buffer: Buffer.from("synthetic-voice-reference"),
  });
  await page.getByRole("button", { name: "Загрузить голос" }).click();
  await expect(page.locator("#voice-reference-status")).toContainText("подтвердите права");
  expect(await page.locator("#voice-reference-file").evaluate((element) =>
    (element as HTMLInputElement).files?.length ?? 0
  )).toBe(0);

  await page.locator("#reference-rights").check();
  await page.locator("#voice-reference-file").setInputFiles({
    name: "voice.webm",
    mimeType: "audio/webm",
    buffer: Buffer.from("synthetic-voice-reference"),
  });
  await page.getByRole("button", { name: "Загрузить голос" }).click();
  await expect(page.locator("#voice-reference-status")).toContainText("raw не хранится");
  await expect(page.locator("#voice-reference-file")).toHaveValue("");
  expect(await page.locator("#voice-reference-file").evaluate((element) =>
    (element as HTMLInputElement).files?.length ?? 0
  )).toBe(0);
  expect(state.voiceReference?.media_type).toBe("audio/webm");
  expect(state.voiceReference?.raw_retained).toBe(false);

  await page.locator("#appearance-reference-file").setInputFiles({
    name: "appearance.png",
    mimeType: "image/png",
    buffer: Buffer.from("synthetic-appearance-reference"),
  });
  await page.getByRole("button", { name: "Загрузить внешность" }).click();
  await expect(page.locator("#appearance-reference-status")).toContainText("raw не хранится");
  await expect(page.locator("#appearance-reference-file")).toHaveValue("");
  expect(await page.locator("#appearance-reference-file").evaluate((element) =>
    (element as HTMLInputElement).files?.length ?? 0
  )).toBe(0);
  expect(state.appearanceReference?.media_type).toBe("image/png");
  expect(state.appearanceReference?.raw_retained).toBe(false);

  await page.getByRole("button", { name: "Очистить" }).first().click();
  await expect(page.locator("#voice-reference-status")).toContainText("Reference очищен");
  expect(state.voiceReference).toBeNull();

  expect(state.apiPaths).toContain("POST /api/references/intake");
  expect(state.apiPaths).toContain("POST /api/references/clear");
});


test("reference upload rejects unknown browser MIME before HTTP", async ({ page }) => {
  const state = initialState();
  await installBrowserFakes(page);
  await installApiFixture(page, state);
  await page.goto("/");

  await page.locator("#reference-rights").check();
  await page.locator("#voice-reference-file").evaluate((element) => {
    const input = element as HTMLInputElement;
    const transfer = new DataTransfer();
    transfer.items.add(new File(["synthetic-voice-reference"], "voice.unknown", { type: "" }));
    input.files = transfer.files;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
  const before = state.apiPaths.filter((path) => path === "POST /api/references/intake").length;
  await page.getByRole("button", { name: "Загрузить голос" }).click();
  await expect(page.locator("#voice-reference-status")).toContainText("REFERENCE_MEDIA_TYPE_UNKNOWN");
  const after = state.apiPaths.filter((path) => path === "POST /api/references/intake").length;
  expect(after).toBe(before);
  expect(await page.locator("#voice-reference-file").evaluate((element) =>
    (element as HTMLInputElement).files?.length ?? 0
  )).toBe(0);
});

test("reference clear releases browser raw source even when backend clear fails", async ({ page }) => {
  const state = initialState();
  await installBrowserFakes(page);
  await installApiFixture(page, state);
  await page.route("**/api/references/clear", async (route) => {
    await json(route, { ok: false, code: "PROVIDER_UNAVAILABLE" }, 503);
  });
  await page.goto("/");

  await page.locator("#voice-reference-file").setInputFiles({
    name: "voice.webm",
    mimeType: "audio/webm",
    buffer: Buffer.from("local-raw-reference"),
  });
  expect(await page.locator("#voice-reference-file").evaluate((element) =>
    (element as HTMLInputElement).files?.length ?? 0
  )).toBe(1);

  await page.getByRole("button", { name: "Очистить" }).first().click();
  await expect(page.locator("#voice-reference-status")).toContainText("PROVIDER_UNAVAILABLE");
  expect(await page.locator("#voice-reference-file").evaluate((element) =>
    (element as HTMLInputElement).files?.length ?? 0
  )).toBe(0);
});


test("stale upload cleanup preserves a newer reference selection", async ({ page }) => {
  const state = initialState();
  await installBrowserFakes(page);
  await installApiFixture(page, state);

  let releaseOldUpload: (() => void) | undefined;
  const oldUploadGate = new Promise<void>((resolve) => {
    releaseOldUpload = resolve;
  });
  await page.route("**/api/references/intake", async (route) => {
    await oldUploadGate;
    await json(route, { ok: false, code: "PROVIDER_TIMEOUT" }, 504);
  });
  await page.goto("/");

  await page.locator("#reference-rights").check();
  const input = page.locator("#voice-reference-file");
  await input.setInputFiles({
    name: "old.webm",
    mimeType: "audio/webm",
    buffer: Buffer.from("old-reference"),
  });
  await page.getByRole("button", { name: "Загрузить голос" }).click();
  await expect(page.locator("#voice-reference-status")).toContainText("Проверяю reference");

  await input.setInputFiles({
    name: "new.webm",
    mimeType: "audio/webm",
    buffer: Buffer.from("new-reference"),
  });
  releaseOldUpload?.();

  await expect(page.locator("#voice-reference-status")).toContainText("PROVIDER_TIMEOUT");
  await expect.poll(async () => input.evaluate((element) =>
    (element as HTMLInputElement).files?.[0]?.name ?? ""
  )).toBe("new.webm");
});

test("reference recorder honors selected microphone device", async ({ page }) => {
  const state = initialState();
  await page.addInitScript(() => {
    const calls = [];
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: {
        getUserMedia: async (constraints) => {
          calls.push(constraints);
          return {
            getTracks: () => [{ stop: () => undefined }],
          };
        },
      },
    });
    class FakeMediaRecorder {
      static isTypeSupported() { return true; }
      state = "inactive";
      mimeType = "audio/webm";
      listeners = new Map();
      constructor() {}
      addEventListener(name, listener) { this.listeners.set(name, listener); }
      start() { this.state = "recording"; }
      stop() {
        this.state = "inactive";
        this.listeners.get("dataavailable")?.({ data: new Blob(["voice"], { type: "audio/webm" }) });
        this.listeners.get("stop")?.();
      }
    }
    Object.defineProperty(window, "MediaRecorder", { configurable: true, value: FakeMediaRecorder });
    window.__referenceMicCalls = calls;
  });
  await installBrowserFakes(page);
  await installApiFixture(page, state);
  await page.goto("/");

  await page.locator("#reference-rights").check();
  await page.locator("#microphone-device").evaluate((element) => {
    const select = element;
    select.append(new Option("Reference mic", "mic-reference"));
    select.value = "mic-reference";
  });
  await page.getByRole("button", { name: "Записать голос (до 15 с)" }).click();

  const calls = await page.evaluate(() => window.__referenceMicCalls);
  expect(calls).toEqual([{ audio: { deviceId: { exact: "mic-reference" } } }]);

  await page.getByRole("button", { name: "Остановить запись" }).click();
  await expect(page.locator("#voice-reference-status")).toContainText("raw не хранится");
  expect(await page.locator("#voice-reference-file").evaluate((element) =>
    (element as HTMLInputElement).files?.length ?? 0
  )).toBe(0);
});

test("owner review, correction, visitor scope and revoke stay connected in one browser journey", async ({ page }) => {
  const state = initialState();
  await installBrowserFakes(page);
  await installApiFixture(page, state);
  await page.goto("/");

  await expect(page.getByRole("button", { name: "Подключить аватар" })).toBeDisabled();
  await expect(page.locator("#readiness-text")).toHaveText("Не готов");
  await expect(page.locator("#readiness-voice")).toHaveText("Не готов");
  await expect(page.locator("#readiness-video")).toHaveText("Не готов");
  await page.getByLabel("Идентификатор Persona").fill("owner-e2e");
  await page.getByRole("button", { name: "Создать Persona" }).click();

  const answers = ["Люблю быстрые итерации", "Мне важна точность", "Предпочитаю спокойный тон"];
  for (const [index, answer] of answers.entries()) {
    await page.getByPlaceholder("Ответьте своими словами").fill(answer);
    await page.getByRole("button", { name: "Сохранить ответ" }).click();
    await expect(page.locator("#persona-progress")).toContainText(`${index + 1}/${questions.length}`);
  }
  const approveButtons = page.getByRole("button", { name: "Подтвердить без изменений" });
  await page.getByRole("button", { name: "Перейти к проверке" }).click();
  await expect(approveButtons).toHaveCount(questions.length);
  for (let index = 0; index < questions.length; index += 1) {
    await approveButtons.first().click();
    await expect(approveButtons).toHaveCount(questions.length - index - 1);
  }
  await page.getByRole("button", { name: "Подтвердить Persona" }).click();
  await expect(page.locator("#persona-progress")).toContainText("версия 2");
  await expect(page.locator("#readiness-text")).toHaveText("Готов");
  await expect(page.locator("#readiness-voice")).toHaveText("Не готов");
  await expect(page.locator("#readiness-video")).toHaveText("Не готов");
  await expect(page.getByRole("button", { name: "Подключить аватар" })).toBeEnabled();

  await page.locator("#consent").check();
  await page.getByRole("button", { name: "Подключить аватар" }).click();
  await expect(page.locator("#status")).toContainText("WebRTC согласован");
  await expect(page.locator("#readiness-voice")).toHaveText("Подготовка…");
  await expect(page.locator("#readiness-video")).toHaveText("Подготовка…");
  await page.getByLabel("Текстовый разговор").fill("Проверка owner scope");
  await page.getByRole("button", { name: "Отправить", exact: true }).click();
  await expect(page.locator("#status")).toContainText("Owner scoped text reply");
  await expect(page.locator("#voice")).toBeEnabled();
  await expect.poll(() => state.textMessages).toEqual(["owner:Проверка owner scope"]);
  await page.getByRole("button", { name: "Закрыть" }).click();
  await expect(page.locator("#status")).toContainText("Сессия закрыта");
  await expect(page.locator("#readiness-voice")).toHaveText("Не готов");
  await expect(page.locator("#readiness-video")).toHaveText("Не готов");

  const ownerClaim = page.getByLabel("Текущее утверждение opinion-working-style");
  await ownerClaim.fill("Предпочитаю короткие циклы проверки");
  await page.getByRole("button", { name: "Сохранить новую редакцию" }).first().click();
  await expect(page.locator("#persona-progress")).toContainText("версия 3");
  await expect(ownerClaim).toHaveValue("Предпочитаю короткие циклы проверки");

  await page.getByLabel("Режим тестовой сессии").selectOption("visitor");
  await expect(page.locator("#persona-panel")).toBeHidden();
  await expect(page.getByLabel("Текстовый разговор")).toBeEnabled();
  await page.getByRole("button", { name: "Подключить аватар" }).click();
  await expect(page.locator("#status")).toContainText("Visitor-сессия WebRTC согласована");
  await page.getByLabel("Текстовый разговор").fill("Проверка visitor scope");
  await page.getByRole("button", { name: "Отправить", exact: true }).click();
  await expect(page.locator("#status")).toContainText("Visitor scoped text reply");
  await page.getByRole("button", { name: "Отозвать доступ" }).click();
  await expect(page.locator("#status")).toContainText("Доступ отозван");
  await page.getByRole("button", { name: "Закрыть" }).click();
  await expect(page.locator("#status")).toContainText("Сессия закрыта");

  expect(state.startAudiences).toEqual(["owner", "visitor"]);
  expect(state.textMessages).toEqual([
    "owner:Проверка owner scope",
    "visitor:Проверка visitor scope",
  ]);
  expect(state.personaVersion).toBe(3);
  expect(state.claims[0]?.revision).toBe(2);
  expect(state.apiPaths).toContain("POST /api/session/revoke");
});

test("LiveKit avatar stays contained and unexpected disconnect closes the backend session", async ({ page }) => {
  const state = initialState();
  state.personaId = "owner-livekit-e2e";
  state.personaVersion = 2;
  state.captureState = "reviewed";
  state.ownerReviewed = true;
  state.transportKind = "live_kit";
  state.claims = [{
    claim_id: "preference-tone",
    statement: "Предпочитаю спокойный тон",
    kind: "preference",
    verification: "verified",
    revision: 1,
    owner_reviewed: true,
  }];

  await installBrowserFakes(page);
  await installLiveKitBrowserFake(page);
  await installApiFixture(page, state);
  await page.goto("/");

  await page.locator("#consent").check();
  await page.getByRole("button", { name: "Подключить аватар" }).click();
  await expect(page.locator("#status")).toContainText("LiveKit согласован");
  await expect(page.locator(".stage")).toHaveClass(/has-video/);
  await expect(page.locator("#readiness-text")).toHaveText("Готов");
  await expect(page.locator("#readiness-video")).toHaveText("Готов");
  await expect(page.locator("#readiness-voice")).toHaveText("Подготовка…");

  const layout = await page.evaluate(() => {
    const stage = document.querySelector<HTMLElement>(".stage");
    const avatar = document.querySelector<HTMLVideoElement>("#avatar");
    if (!stage || !avatar) throw new Error("missing realtime stage");
    const stageRect = stage.getBoundingClientRect();
    const avatarRect = avatar.getBoundingClientRect();
    return {
      bodyOverflow: document.documentElement.scrollWidth > window.innerWidth,
      objectFit: getComputedStyle(avatar).objectFit,
      stageWidth: stageRect.width,
      stageHeight: stageRect.height,
      avatarWidth: avatarRect.width,
      avatarHeight: avatarRect.height,
    };
  });
  expect(layout.bodyOverflow).toBe(false);
  expect(layout.objectFit).toBe("contain");
  expect(layout.avatarWidth).toBeLessThanOrEqual(layout.stageWidth);
  expect(layout.avatarHeight).toBeLessThanOrEqual(layout.stageHeight);

  await page.evaluate(() => {
    const fakeWindow = window as typeof window & { __vprFakeLiveKitDisconnect?: () => void };
    fakeWindow.__vprFakeLiveKitDisconnect?.();
  });
  await expect(page.locator("#status")).toContainText("evidence snapshot сохранён");
  await expect.poll(() => state.sessionState).toBe("closed");
  await expect.poll(() => state.apiPaths.includes("POST /api/evidence/session/export")).toBe(true);
  expect(state.apiPaths).toContain("POST /api/session/close");
});

test("active visitor recovery does not request owner-only persona endpoints", async ({ page }) => {
  const state = initialState();
  state.personaId = "owner-e2e";
  state.personaVersion = 3;
  state.captureState = "reviewed";
  state.ownerReviewed = true;
  state.sessionState = "active";
  state.sessionAudience = "visitor";
  state.avatarOpen = true;
  state.claims = [{
    claim_id: "opinion-working-style",
    statement: "Секретный owner-reviewed текст",
    kind: "opinion",
    verification: "verified",
    revision: 2,
    owner_reviewed: true,
  }];

  await installBrowserFakes(page);
  await installApiFixture(page, state);
  await page.goto("/");

  await expect(page.locator("#persona-panel")).toBeHidden();
  await expect(page.locator("#status")).toContainText("Найдена незакрытая сессия");
  expect(state.apiPaths).not.toContain("GET /api/persona/capture");
  expect(state.apiPaths).not.toContain("POST /api/persona/reviewed");
});
