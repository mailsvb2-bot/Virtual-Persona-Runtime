import {
  expect,
  test,
  type APIRequestContext,
  type Page,
} from "@playwright/test";

const ownerLabUrl = "http://127.0.0.1:18789";
const providerUrl = "http://127.0.0.1:18790";
const ownerAnswers = [
  "Я создаю виртуальных персонажей",
  "Отвечай кратко и спокойно",
  "Точность важнее уверенного выдумывания",
];

const csrfHeaders = (csrf: string) => ({
  "content-type": "application/json",
  origin: ownerLabUrl,
  "x-vpr-csrf": csrf,
});

const postJson = async (
  request: APIRequestContext,
  csrf: string,
  path: string,
  data: unknown,
) => request.post(`${ownerLabUrl}${path}`, {
  headers: csrfHeaders(csrf),
  data,
});
const setupReviewedPersona = async (
  request: APIRequestContext,
  csrf: string,
): Promise<void> => {
  const created = await postJson(
    request,
    csrf,
    "/api/persona/create",
    { persona_id: "owner-voice-e2e" },
  );
  expect(created.status()).toBe(201);

  for (const answer of ownerAnswers) {
    const captured = await postJson(
      request,
      csrf,
      "/api/persona/capture/answer",
      { answer },
    );
    expect(captured.ok()).toBeTruthy();
  }

  const finished = await postJson(
    request,
    csrf,
    "/api/persona/capture/finish",
    {},
  );
  expect(finished.ok()).toBeTruthy();
  const snapshot = await finished.json() as {
    claims: Array<{ claim_id: string }>;
  };
  for (const claim of snapshot.claims) {
    const approved = await postJson(
      request,
      csrf,
      "/api/persona/claims/approve",
      { claim_id: claim.claim_id },
    );
    expect(approved.ok()).toBeTruthy();
  }

  const reviewed = await postJson(
    request,
    csrf,
    "/api/persona/review/complete",
    {},
  );
  expect(reviewed.ok()).toBeTruthy();
};

const installBrowserAudioFakes = async (page: Page): Promise<void> => {
  await page.addInitScript(() => {
    let remoteSpeech = false;
    let trackSequence = 0;
    const realFetch = window.fetch.bind(window);
    window.fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const target = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      if (target.endsWith("/api/avatar/start")) remoteSpeech = false;
      if (!target.endsWith("/api/voice/turn")) return realFetch(input, init);
      remoteSpeech = true;
      const response = await realFetch(input, init);
      await new Promise((resolve) => window.setTimeout(resolve, 80));
      return response;
    };

    class FakeTrack {
      id: string;
      kind: "audio";
      constructor(kind: "audio" = "audio") {
        trackSequence += 1;
        this.id = `fake-track-${trackSequence}`;
        this.kind = kind;
      }
      stop(): void {}
    }

    class FakeMediaStream {
      private tracks: FakeTrack[];
      constructor(tracks: FakeTrack[] = []) { this.tracks = [...tracks]; }
      getTracks(): FakeTrack[] { return [...this.tracks]; }
      addTrack(track: FakeTrack): void { this.tracks.push(track); }
    }

    Object.defineProperty(window, "MediaStream", {
      configurable: true,
      value: FakeMediaStream,
    });
    Object.defineProperty(HTMLMediaElement.prototype, "srcObject", {
      configurable: true,
      get() { return (this as HTMLMediaElement & { __vprSrc?: unknown }).__vprSrc ?? null; },
      set(value: unknown) { (this as HTMLMediaElement & { __vprSrc?: unknown }).__vprSrc = value; },
    });
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: { getUserMedia: async () => new FakeMediaStream([new FakeTrack()]) },
    });

    class FakeAnalyser {
      fftSize = 256;
      connect(): void {}
      disconnect(): void {}
      getFloatTimeDomainData(samples: Float32Array): void {
        samples.fill(remoteSpeech ? 0.12 : 0.0005);
      }
    }
    class FakeGain {
      gain = { value: 1 };
      connect(): void {}
      disconnect(): void {}
    }
    class FakeAudioContext {
      sampleRate = 48_000;
      destination = {};
      audioWorklet = { addModule: async () => undefined };
      async resume(): Promise<void> {}
      async close(): Promise<void> {}
      createMediaStreamSource(): { connect: () => void; disconnect: () => void } {
        return { connect: () => undefined, disconnect: () => undefined };
      }
      createAnalyser(): FakeAnalyser { return new FakeAnalyser(); }
      createGain(): FakeGain { return new FakeGain(); }
    }

    class FakeAudioWorkletNode {
      port: { onmessage: ((event: { data: ArrayBuffer }) => void) | null } = {
        onmessage: null,
      };
      connect(): void {
        const samples = new Float32Array(4_800);
        samples.fill(0.2);
        queueMicrotask(() => this.port.onmessage?.({ data: samples.buffer }));
      }
      disconnect(): void {}
    }

    class FakePeerConnection {
      connectionState = "new";
      ontrack: ((event: { track: FakeTrack }) => void) | null = null;
      onconnectionstatechange: (() => void) | null = null;
      onicecandidate: ((event: unknown) => void) | null = null;
      async setRemoteDescription(): Promise<void> {
        queueMicrotask(() => this.ontrack?.({ track: new FakeTrack("audio") }));
      }
      async createAnswer(): Promise<{ type: "answer"; sdp: string }> {
        return { type: "answer", sdp: "v=0 voice-browser-answer" };
      }
      async setLocalDescription(): Promise<void> {}
      async getStats(): Promise<Map<string, object>> {
        return new Map([
          ["audio", { type: "inbound-rtp", kind: "audio", packetsReceived: 20, estimatedPlayoutTimestamp: 1_000 }],
          ["video", { type: "inbound-rtp", kind: "video", packetsReceived: 20, estimatedPlayoutTimestamp: 1_060 }],
        ]);
      }
      close(): void { this.connectionState = "closed"; }
    }

    Object.defineProperty(window, "AudioContext", { configurable: true, value: FakeAudioContext });
    Object.defineProperty(window, "AudioWorkletNode", { configurable: true, value: FakeAudioWorkletNode });
    Object.defineProperty(window, "RTCPeerConnection", { configurable: true, value: FakePeerConnection });
  });
};

const recordVoiceTurn = async (
  page: Page,
  transcript: string,
  reply: string,
): Promise<void> => {
  const voice = page.locator("#voice");
  await voice.click();
  await expect(voice).toHaveText("Остановить и отправить");
  await voice.click();
  await expect(page.locator("#status")).toContainText(`Вы: ${transcript}`);
  await expect(page.locator("#status")).toContainText(`Ответ: ${reply}`);
};

test("owner and visitor voice turns cross the real backend with different context scopes", async ({
  page,
  request,
}) => {
  const bootstrap = await request.get(`${ownerLabUrl}/api/bootstrap`);
  expect(bootstrap.ok()).toBeTruthy();
  const csrf = String((await bootstrap.json()).csrf_token);
  await setupReviewedPersona(request, csrf);

  await installBrowserAudioFakes(page);
  await page.goto("/");
  await expect(page.locator("#persona-progress")).toContainText("версия 2");
  await page.getByRole("checkbox").check();
  await page.getByRole("button", { name: "Подключить аватар" }).click();
  await expect(page.locator("#status")).toContainText("WebRTC согласован");
  await expect(page.locator("#voice")).toBeEnabled();

  await recordVoiceTurn(
    page,
    "Привет из браузера",
    "Голосовой ответ владельцу",
  );
  const ownerEvidence = await request.get(`${ownerLabUrl}/api/evidence/session`);
  expect(ownerEvidence.ok()).toBeTruthy();
  const ownerEvidenceJson = await ownerEvidence.json() as {
    participant_role: "owner" | "visitor";
    canonical_playback_proven: boolean;
    av_sync_proven: boolean;
    av_sync_samples: Array<{ request_sequence: number; sample_sequence: number; reference: string; absolute_offset_millis: number }>;
    voice_attempts: Array<{
      request_sequence: number;
      canonical_turn_sequence: number;
      canonical_output_sequence: number;
      canonical_playback_confirmed: boolean;
      status: string;
    }>;
    media_events: Array<{ request_sequence: number | null; kind: string }>;
  };
  expect(ownerEvidenceJson.participant_role).toBe("owner");
  expect(ownerEvidenceJson.canonical_playback_proven).toBeTruthy();
  expect(ownerEvidenceJson.av_sync_proven).toBeTruthy();
  expect(ownerEvidenceJson.av_sync_samples).toHaveLength(3);
  expect(ownerEvidenceJson.av_sync_samples.every((sample) =>
    sample.request_sequence === 1
      && sample.reference === "web_rtc_estimated_playout_timestamp"
      && sample.absolute_offset_millis === 60
  )).toBeTruthy();
  expect(ownerEvidenceJson.voice_attempts).toMatchObject([{
    request_sequence: 1,
    canonical_playback_confirmed: true,
    status: "completed",
  }]);
  expect(ownerEvidenceJson.voice_attempts[0]?.canonical_turn_sequence).toBeGreaterThan(0);
  expect(ownerEvidenceJson.voice_attempts[0]?.canonical_output_sequence).toBeGreaterThan(0);
  expect(ownerEvidenceJson.media_events).toContainEqual({
    request_sequence: 1,
    kind: "audio_started",
    elapsed_millis: expect.any(Number),
  });

  await page.getByRole("button", { name: "Закрыть" }).click();
  await expect(page.locator("#status")).toContainText("Сессия закрыта");
  await page.getByLabel("Режим тестовой сессии").selectOption("visitor");
  await page.getByRole("button", { name: "Подключить аватар" }).click();
  await expect(page.locator("#status")).toContainText("Visitor-сессия WebRTC согласована");

  await recordVoiceTurn(
    page,
    "Что думает владелец?",
    "В visitor scope нет подтверждённых данных владельца",
  );

  const visitorEvidence = await request.get(`${ownerLabUrl}/api/evidence/session`);
  expect(visitorEvidence.ok()).toBeTruthy();
  const visitorEvidenceJson = await visitorEvidence.json() as {
    participant_role: "owner" | "visitor";
    canonical_playback_proven: boolean;
    av_sync_proven: boolean;
    av_sync_samples: Array<{ request_sequence: number; sample_sequence: number; reference: string; absolute_offset_millis: number }>;
    voice_attempts: Array<{
      request_sequence: number;
      canonical_turn_sequence: number;
      canonical_output_sequence: number;
      canonical_playback_confirmed: boolean;
      status: string;
    }>;
    media_events: Array<{ request_sequence: number | null; kind: string; elapsed_millis: number }>;
  };
  expect(visitorEvidenceJson.participant_role).toBe("visitor");
  expect(visitorEvidenceJson.canonical_playback_proven).toBeTruthy();
  expect(visitorEvidenceJson.av_sync_proven).toBeTruthy();
  expect(visitorEvidenceJson.av_sync_samples).toHaveLength(3);
  expect(visitorEvidenceJson.av_sync_samples.every((sample) =>
    sample.request_sequence === 1
      && sample.reference === "web_rtc_estimated_playout_timestamp"
      && sample.absolute_offset_millis === 60
  )).toBeTruthy();
  expect(visitorEvidenceJson.voice_attempts).toMatchObject([{
    request_sequence: 1,
    canonical_playback_confirmed: true,
    status: "completed",
  }]);
  expect(visitorEvidenceJson.voice_attempts[0]?.canonical_turn_sequence).toBeGreaterThan(0);
  expect(visitorEvidenceJson.voice_attempts[0]?.canonical_output_sequence).toBeGreaterThan(0);
  expect(visitorEvidenceJson.media_events.some((event) =>
    event.request_sequence === 1 && event.kind === "audio_started"
  )).toBeTruthy();

  await page.getByRole("button", { name: "Отозвать доступ" }).click();
  await expect(page.locator("#status")).toContainText("Доступ отозван");
  await page.getByRole("button", { name: "Закрыть" }).click();
  await expect(page.locator("#status")).toContainText("Сессия закрыта");

  const providerState = await request.get(`${providerUrl}/__state`);
  expect(providerState.ok()).toBeTruthy();
  const { requests } = await providerState.json() as {
    requests: Array<{
      kind: "stt" | "llm" | "avatar";
      method: string;
      path: string;
      authorization: string | null;
      contentType: string | null;
      bodyLength: number;
      bodyText: string;
    }>;
  };

  const stt = requests.filter((entry) => entry.kind === "stt");
  const llm = requests.filter((entry) => entry.kind === "llm");
  const avatar = requests.filter((entry) => entry.kind === "avatar");
  expect(stt).toHaveLength(2);
  expect(llm).toHaveLength(2);
  expect(avatar.filter((entry) => entry.path.endsWith("/streams"))).toHaveLength(2);
  expect(avatar.filter((entry) => entry.path.endsWith("/sdp"))).toHaveLength(2);
  expect(avatar.filter((entry) => entry.method === "DELETE")).toHaveLength(2);

  expect(stt.every((entry) => entry.authorization === "Bearer voice-stt-e2e-secret")).toBeTruthy();
  expect(stt.every((entry) => entry.contentType?.startsWith("multipart/form-data"))).toBeTruthy();
  expect(stt.every((entry) => entry.bodyLength > 3_000)).toBeTruthy();
  expect(llm.every((entry) => entry.authorization === "Bearer voice-llm-e2e-secret")).toBeTruthy();
  expect(llm[0]?.bodyText).toContain("Привет из браузера");
  for (const ownerAnswer of ownerAnswers) {
    expect(llm[0]?.bodyText).toContain(ownerAnswer);
    expect(llm[1]?.bodyText).not.toContain(ownerAnswer);
  }
  expect(llm[1]?.bodyText).toContain("Что думает владелец?");
  expect(llm[1]?.bodyText).toContain("Visitor permissions do not expose owner-reviewed personal context");

  expect(avatar.every((entry) => entry.authorization === "Basic voice-avatar-e2e-secret")).toBeTruthy();
  const ownerSpeech = avatar.find((entry) =>
    entry.method === "POST" && entry.path.endsWith("/stream-1")
  );
  const visitorSpeech = avatar.find((entry) =>
    entry.method === "POST" && entry.path.endsWith("/stream-2")
  );
  expect(ownerSpeech?.bodyText).toContain("Голосовой ответ владельцу");
  expect(visitorSpeech?.bodyText).toContain("В visitor scope нет подтверждённых данных владельца");
});
