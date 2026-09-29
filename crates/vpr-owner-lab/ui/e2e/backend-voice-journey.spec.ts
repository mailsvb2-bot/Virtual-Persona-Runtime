import {
  expect,
  test,
  type APIRequestContext,
  type Page,
} from "@playwright/test";

import { installProviderAutoConnect } from "./provider-bootstrap.js";

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

const prepareBrowserRuntimeFakes = async (page: Page): Promise<void> => {
  await page.addInitScript(() => {
    let remoteSpeech = false;
    let trackSequence = 0;
    let playbackSequence = 0;
    let activePeer: {
      connectionState: string;
      onconnectionstatechange: (() => void) | null;
    } | null = null;
    let providerDataChannel: {
      label: string;
      readyState: "open" | "closed";
      onopen: (() => void) | null;
      onclose: (() => void) | null;
      onmessage: ((event: { data: string }) => void) | null;
      send: (payload: string) => void;
      close: () => void;
    } | null = null;

    const requestedMicrophones: string[] = [];
    const interruptPayloads: string[] = [];
    const deviceChangeListeners: Array<() => void> = [];
    let testDevicesReady = false;
    const fakeWindow = window as unknown as {
      __vprRequestedMicrophones?: string[];
      __vprInterruptPayloads?: string[];
      __vprSetPeerConnectionState?: (state: "connected" | "disconnected" | "failed") => void;
      __vprCreateMicStream?: (deviceId: string) => MediaStream;
      __vprStartVoicePlayback?: () => Promise<void>;
      __vprTestMediaRuntime?: unknown;
    };
    fakeWindow.__vprRequestedMicrophones = requestedMicrophones;
    fakeWindow.__vprInterruptPayloads = interruptPayloads;

    class FakeTrack {
      id: string;
      kind: "audio";
      constructor(kind: "audio" = "audio", readonly deviceId = "builtin-mic") {
        trackSequence += 1;
        this.id = `fake-track-${trackSequence}`;
        this.kind = kind;
      }
      stop(): void {}
      getSettings(): MediaTrackSettings { return { deviceId: this.deviceId }; }
    }

    class FakeMediaStream {
      private tracks: FakeTrack[];
      constructor(tracks: FakeTrack[] = []) { this.tracks = [...tracks]; }
      getTracks(): FakeTrack[] { return [...this.tracks]; }
      getAudioTracks(): FakeTrack[] { return this.tracks.filter((track) => track.kind === "audio"); }
      addTrack(track: FakeTrack): void { this.tracks.push(track); }
    }

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
      sampleRate: number;
      constructor(options?: AudioContextOptions) {
        this.sampleRate = options?.sampleRate ?? 48_000;
      }
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

    class FakePeerConnection {
      connectionState = "new";
      ontrack: ((event: { track: FakeTrack }) => void) | null = null;
      onconnectionstatechange: (() => void) | null = null;
      onicecandidate: ((event: unknown) => void) | null = null;

      constructor() {
        activePeer = this;
      }

      createDataChannel(label: string) {
        const channel = {
          label,
          readyState: "open" as "open" | "closed",
          onopen: null as (() => void) | null,
          onclose: null as (() => void) | null,
          onmessage: null as ((event: { data: string }) => void) | null,
          send(payload: string): void {
            interruptPayloads.push(payload);
            remoteSpeech = false;
            queueMicrotask(() => channel.onmessage?.({ data: "stream/done:{}" }));
          },
          close(): void {
            channel.readyState = "closed";
            channel.onclose?.();
          },
        };
        providerDataChannel = channel;
        queueMicrotask(() => channel.onopen?.());
        return channel as unknown as RTCDataChannel;
      }

      async setRemoteDescription(): Promise<void> {
        queueMicrotask(() => this.ontrack?.({ track: new FakeTrack("audio") }));
      }
      async createAnswer(): Promise<{ type: "answer"; sdp: string }> {
        return { type: "answer", sdp: "v=0 voice-browser-answer" };
      }
      async setLocalDescription(): Promise<void> {
        // A local answer does not mean ICE/DTLS is connected. The provider harness advances
        // connection state only after the backend has accepted /api/avatar/answer.
      }
      async getStats(): Promise<Map<string, object>> {
        return new Map([
          ["audio", { type: "inbound-rtp", kind: "audio", packetsReceived: 20, estimatedPlayoutTimestamp: 1_000 }],
          ["video", { type: "inbound-rtp", kind: "video", packetsReceived: 20, estimatedPlayoutTimestamp: 1_060 }],
        ]);
      }
      close(): void { this.connectionState = "closed"; }
    }

    fakeWindow.__vprSetPeerConnectionState = (state) => {
      if (!activePeer) throw new Error("NO_ACTIVE_PEER");
      activePeer.connectionState = state;
      activePeer.onconnectionstatechange?.();
    };

    const mediaDevices = {
      enumerateDevices: async () => {
        if (!testDevicesReady) return [];
        return [
          { deviceId: "builtin-mic", kind: "audioinput", label: "Встроенный микрофон", groupId: "g1", toJSON: () => ({}) },
          { deviceId: "headset-mic", kind: "audioinput", label: "Микрофон гарнитуры", groupId: "g2", toJSON: () => ({}) },
        ];
      },
      addEventListener: (type: string, listener: EventListenerOrEventListenerObject) => {
        if (type !== "devicechange") return;
        deviceChangeListeners.push(() => {
          if (typeof listener === "function") listener(new Event("devicechange"));
          else listener.handleEvent(new Event("devicechange"));
        });
      },
    };

    fakeWindow.__vprCreateMicStream = (deviceId: string) =>
      new FakeMediaStream([new FakeTrack("audio", deviceId)]) as unknown as MediaStream;
    fakeWindow.__vprStartVoicePlayback = async () => {
      remoteSpeech = true;
      playbackSequence += 1;
      providerDataChannel?.onmessage?.({
        data: `stream/started:${JSON.stringify({ metadata: { videoId: `video-${playbackSequence}` } })}`,
      });
      await new Promise((resolve) => window.setTimeout(resolve, 80));
    };

    fakeWindow.__vprTestMediaRuntime = {
      afterApiResponse: async (path: string) => {
        if (path.endsWith("/api/avatar/start")) {
          remoteSpeech = false;
          testDevicesReady = true;
          queueMicrotask(() => deviceChangeListeners.forEach((listener) => listener()));
          return;
        }
        if (path.endsWith("/api/avatar/answer")) {
          // The backend has consumed the SDP answer and the application has consumed the
          // successful response. Only now may the synthetic transport become connected.
          fakeWindow.__vprSetPeerConnectionState?.("connected");
        }
      },
      mediaDevices,
      AudioContext: FakeAudioContext,
      MediaStream: FakeMediaStream,
      RTCPeerConnection: FakePeerConnection,
      setSrcObject(element: HTMLMediaElement, value: unknown) {
        (element as HTMLMediaElement & { __vprTestSrcObject?: unknown }).__vprTestSrcObject = value;
      },
    };

  });
};

const prepareVoiceCaptureFakes = async (page: Page): Promise<void> => {
  await page.evaluate(() => {
    const fakeWindow = window as unknown as {
      __vprRequestedMicrophones?: string[];
      __vprCreateMicStream?: (deviceId: string) => MediaStream;
      __vprStartVoicePlayback?: () => Promise<void>;
      __vprTestMediaRuntime?: {
        afterApiResponse?: (path: string) => void | Promise<void>;
        mediaDevices?: Record<string, unknown>;
        AudioWorkletNode?: unknown;
      };
    };
    const runtime = fakeWindow.__vprTestMediaRuntime;
    if (!runtime || !fakeWindow.__vprCreateMicStream || !fakeWindow.__vprStartVoicePlayback) {
      throw new Error("VOICE_TEST_RUNTIME_NOT_READY");
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

    const mediaDevices = runtime.mediaDevices as {
      enumerateDevices?: () => Promise<MediaDeviceInfo[]>;
      addEventListener?: (type: string, listener: EventListenerOrEventListenerObject) => void;
      getUserMedia?: (constraints: MediaStreamConstraints) => Promise<MediaStream>;
    };
    mediaDevices.getUserMedia = async (constraints: MediaStreamConstraints) => {
      const audio = typeof constraints.audio === "object" && constraints.audio !== null
        ? constraints.audio
        : {};
      const requested = typeof audio.deviceId === "object"
        && audio.deviceId !== null
        && "exact" in audio.deviceId
        ? String(audio.deviceId.exact)
        : "builtin-mic";
      fakeWindow.__vprRequestedMicrophones?.push(requested);
      return fakeWindow.__vprCreateMicStream?.(requested) as MediaStream;
    };
    runtime.AudioWorkletNode = FakeAudioWorkletNode;
    const previousHook = runtime.afterApiResponse;
    runtime.afterApiResponse = async (path: string) => {
      await previousHook?.(path);
      if (path.endsWith("/api/voice/input/finish")) {
        await fakeWindow.__vprStartVoicePlayback?.();
      }
    };
  });
};

const recordTextTurn = async (
  page: Page,
  input: string,
  reply: string,
): Promise<void> => {
  await page.getByLabel("Текстовый разговор").fill(input);
  await page.getByRole("button", { name: "Отправить", exact: true }).click();
  await expect(page.locator("#status")).toContainText(`Ответ: ${reply}`);
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

const waitForCanonicalPlaybackEvidence = async (
  request: APIRequestContext,
): Promise<void> => {
  await expect.poll(async () => {
    const evidence = await request.get(`${ownerLabUrl}/api/evidence/session`);
    if (!evidence.ok()) {
      return false;
    }
    const snapshot = await evidence.json() as {
      canonical_playback_proven: boolean;
      av_sync_proven: boolean;
      voice_attempts: Array<{
        status: string;
        canonical_playback_confirmed: boolean;
      }>;
    };
    return snapshot.canonical_playback_proven
      && snapshot.av_sync_proven
      && snapshot.voice_attempts.some((attempt) =>
        attempt.status === "completed" && attempt.canonical_playback_confirmed
      );
  }).toBeTruthy();
};

test("owner and visitor voice turns cross the real backend with different context scopes", async ({
  page,
  request,
}) => {
  const bootstrap = await request.get(`${ownerLabUrl}/api/bootstrap`);
  expect(bootstrap.ok()).toBeTruthy();
  const csrf = String((await bootstrap.json()).csrf_token);
  await setupReviewedPersona(request, csrf);

  await prepareBrowserRuntimeFakes(page);
  await installProviderAutoConnect(page);
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-vpr-provider-auto-connect", "clicked");
  await expect(page.locator("#persona-progress")).toContainText("версия 2");
  await expect(page.locator("#status")).toContainText("WebRTC согласован");
  await prepareVoiceCaptureFakes(page);
  await expect(page.locator("#voice")).toBeEnabled();
  await expect(page.getByRole("button", { name: "Отправить", exact: true })).toBeEnabled();
  const microphone = page.getByLabel("Микрофон");
  await expect(microphone.locator("option")).toHaveCount(3);
  await microphone.selectOption("headset-mic");

  await recordTextTurn(
    page,
    "Текстовый вопрос владельца",
    "Текстовый ответ владельцу",
  );
  await recordVoiceTurn(
    page,
    "Привет из браузера",
    "Голосовой ответ владельцу",
  );
  await waitForCanonicalPlaybackEvidence(request);
  await expect.poll(() => page.evaluate(
    () => (window as unknown as { __vprRequestedMicrophones?: string[] }).__vprRequestedMicrophones ?? [],
  )).toContain("headset-mic");
  const interrupt = page.getByRole("button", { name: "Прервать", exact: true });
  await expect(interrupt).toBeEnabled();
  await interrupt.click();
  await expect.poll(async () => {
    const evidence = await request.get(`${ownerLabUrl}/api/evidence/session`);
    const snapshot = await evidence.json() as {
      media_events: Array<{ kind: string }>;
    };
    return snapshot.media_events.some((event) => event.kind === "interruption_stopped");
  }).toBeTruthy();
  const interruptPayloads = await page.evaluate(
    () => (window as unknown as { __vprInterruptPayloads?: string[] }).__vprInterruptPayloads ?? [],
  );
  expect(interruptPayloads).toHaveLength(1);
  expect(JSON.parse(interruptPayloads[0] ?? "{}")).toMatchObject({
    type: "stream/interrupt",
    videoId: "video-1",
  });
  expect(Number(JSON.parse(interruptPayloads[0] ?? "{}").timestamp)).toBeGreaterThan(0);

  await page.evaluate(() => {
    (window as unknown as {
      __vprSetPeerConnectionState: (state: "disconnected") => void;
    }).__vprSetPeerConnectionState("disconnected");
  });
  await page.waitForTimeout(25);
  await page.evaluate(() => {
    (window as unknown as {
      __vprSetPeerConnectionState: (state: "connected") => void;
    }).__vprSetPeerConnectionState("connected");
  });
  await expect.poll(async () => {
    const evidence = await request.get(`${ownerLabUrl}/api/evidence/session`);
    const snapshot = await evidence.json() as {
      media_events: Array<{ kind: string; elapsed_millis: number }>;
    };
    return snapshot.media_events.find((event) => event.kind === "reconnect_restored")
      ?.elapsed_millis ?? 0;
  }).toBeGreaterThan(0);

  const ownerEvidence = await request.get(`${ownerLabUrl}/api/evidence/session`);
  expect(ownerEvidence.ok()).toBeTruthy();
  const ownerEvidenceJson = await ownerEvidence.json() as {
    participant_role: "owner" | "visitor";
    canonical_playback_proven: boolean;
    av_sync_proven: boolean;
    av_sync_samples: Array<{ request_sequence: number; sample_sequence: number; reference: string; absolute_offset_millis: number }>;
    text_attempts: Array<{
      request_sequence: number;
      canonical_turn_sequence: number;
      canonical_output_sequence: number;
      status: string;
      first_meaningful_response_millis: number;
      server_total_millis: number;
    }>;
    voice_attempts: Array<{
      request_sequence: number;
      canonical_turn_sequence: number;
      canonical_output_sequence: number;
      canonical_playback_confirmed: boolean;
      status: string;
      llm_millis: number;
      llm_first_meaningful_millis: number;
    }>;
    media_events: Array<{ request_sequence: number | null; kind: string }>;
  };
  expect(ownerEvidenceJson.participant_role).toBe("owner");
  expect(ownerEvidenceJson.canonical_playback_proven).toBeTruthy();
  expect(ownerEvidenceJson.av_sync_proven).toBeTruthy();
  expect(ownerEvidenceJson.av_sync_samples).toHaveLength(3);
  expect(ownerEvidenceJson.av_sync_samples.map((sample) => sample.sample_sequence)).toEqual([1, 2, 3]);
  expect(ownerEvidenceJson.text_attempts).toMatchObject([{
    request_sequence: 1,
    status: "completed",
  }]);
  expect(ownerEvidenceJson.text_attempts[0]?.canonical_turn_sequence).toBeGreaterThan(0);
  expect(ownerEvidenceJson.text_attempts[0]?.canonical_output_sequence).toBeGreaterThan(0);
  expect(ownerEvidenceJson.text_attempts[0]?.first_meaningful_response_millis).toBeLessThanOrEqual(
    ownerEvidenceJson.text_attempts[0]?.server_total_millis ?? -1,
  );
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
  expect(ownerEvidenceJson.voice_attempts[0]?.llm_first_meaningful_millis).toBeLessThanOrEqual(
    ownerEvidenceJson.voice_attempts[0]?.llm_millis ?? -1,
  );
  expect(ownerEvidenceJson.media_events).toContainEqual({
    request_sequence: 1,
    kind: "audio_started",
    elapsed_millis: expect.any(Number),
  });
  expect(ownerEvidenceJson.media_events).toContainEqual({
    request_sequence: 1,
    kind: "interruption_stopped",
    elapsed_millis: expect.any(Number),
  });
  expect(ownerEvidenceJson.media_events).toContainEqual({
    request_sequence: null,
    kind: "reconnect_restored",
    elapsed_millis: expect.any(Number),
  });

  await page.getByLabel("Текстовый разговор").fill("Спровоцируй отказ провайдера");
  await page.getByRole("button", { name: "Отправить", exact: true }).click();
  await expect(page.locator("#status")).toContainText("PROVIDER_UNAVAILABLE");
  await expect(page.locator("#persona-progress")).toContainText("версия 2");

  await recordTextTurn(
    page,
    "Восстановление после отказа",
    "Ответ после восстановления",
  );
  await expect(page.locator("#persona-progress")).toContainText("версия 2");

  const recoveredEvidence = await request.get(`${ownerLabUrl}/api/evidence/session`);
  expect(recoveredEvidence.ok()).toBeTruthy();
  const recoveredEvidenceJson = await recoveredEvidence.json() as {
    participant_role: "owner" | "visitor";
    text_attempts: Array<{
      request_sequence: number;
      status: string;
      failure_code: string | null;
    }>;
  };
  expect(recoveredEvidenceJson.participant_role).toBe("owner");
  expect(recoveredEvidenceJson.text_attempts).toMatchObject([
    { request_sequence: 1, status: "completed", failure_code: null },
    { request_sequence: 2, status: "failed", failure_code: "PROVIDER_UNAVAILABLE" },
    { request_sequence: 3, status: "completed", failure_code: null },
  ]);

  await page.getByRole("button", { name: "Закрыть" }).click();
  await expect(page.locator("#status")).toContainText("Сессия закрыта");
  const ownerEvidenceDownloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: "Скачать evidence snapshot" }).click();
  const ownerEvidenceDownload = await ownerEvidenceDownloadPromise;
  expect(ownerEvidenceDownload.suggestedFilename()).toBe("session-1-owner.json");
  await page.getByLabel("Режим тестовой сессии").selectOption("visitor");
  await page.getByRole("button", { name: "Подключить аватар" }).click();
  await expect(page.locator("#status")).toContainText("Visitor-сессия WebRTC согласована");

  await recordTextTurn(
    page,
    "Текстовый вопрос visitor",
    "Текстовый ответ visitor",
  );
  await recordVoiceTurn(
    page,
    "Что думает владелец?",
    "В visitor scope нет подтверждённых данных владельца",
  );
  await waitForCanonicalPlaybackEvidence(request);

  const visitorEvidence = await request.get(`${ownerLabUrl}/api/evidence/session`);
  expect(visitorEvidence.ok()).toBeTruthy();
  const visitorEvidenceJson = await visitorEvidence.json() as {
    participant_role: "owner" | "visitor";
    canonical_playback_proven: boolean;
    av_sync_proven: boolean;
    av_sync_samples: Array<{ request_sequence: number; sample_sequence: number; reference: string; absolute_offset_millis: number }>;
    text_attempts: Array<{
      request_sequence: number;
      canonical_turn_sequence: number;
      canonical_output_sequence: number;
      status: string;
      first_meaningful_response_millis: number;
      server_total_millis: number;
    }>;
    voice_attempts: Array<{
      request_sequence: number;
      canonical_turn_sequence: number;
      canonical_output_sequence: number;
      canonical_playback_confirmed: boolean;
      status: string;
      llm_millis: number;
      llm_first_meaningful_millis: number;
    }>;
    media_events: Array<{ request_sequence: number | null; kind: string; elapsed_millis: number }>;
  };
  expect(visitorEvidenceJson.participant_role).toBe("visitor");
  expect(visitorEvidenceJson.canonical_playback_proven).toBeTruthy();
  expect(visitorEvidenceJson.av_sync_proven).toBeTruthy();
  expect(visitorEvidenceJson.av_sync_samples).toHaveLength(3);
  expect(visitorEvidenceJson.av_sync_samples.map((sample) => sample.sample_sequence)).toEqual([1, 2, 3]);
  expect(visitorEvidenceJson.text_attempts).toMatchObject([{
    request_sequence: 1,
    status: "completed",
  }]);
  expect(visitorEvidenceJson.text_attempts[0]?.canonical_turn_sequence).toBeGreaterThan(0);
  expect(visitorEvidenceJson.text_attempts[0]?.canonical_output_sequence).toBeGreaterThan(0);
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
  expect(visitorEvidenceJson.voice_attempts[0]?.llm_first_meaningful_millis).toBeLessThanOrEqual(
    visitorEvidenceJson.voice_attempts[0]?.llm_millis ?? -1,
  );
  expect(visitorEvidenceJson.media_events.some((event) =>
    event.request_sequence === 1 && event.kind === "audio_started"
  )).toBeTruthy();

  await page.getByRole("button", { name: "Отозвать доступ" }).click();
  await expect(page.locator("#status")).toContainText("Доступ отозван");
  await page.getByRole("button", { name: "Закрыть" }).click();
  await expect(page.locator("#status")).toContainText("Сессия закрыта");
  const visitorEvidenceDownloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: "Скачать evidence snapshot" }).click();
  const visitorEvidenceDownload = await visitorEvidenceDownloadPromise;
  expect(visitorEvidenceDownload.suggestedFilename()).toBe("session-2-visitor.json");

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
  expect(llm).toHaveLength(6);
  expect(avatar.filter((entry) => entry.path.endsWith("/streams"))).toHaveLength(2);
  expect(avatar.filter((entry) => entry.path.endsWith("/sdp"))).toHaveLength(2);
  expect(avatar.filter((entry) => entry.method === "DELETE")).toHaveLength(2);

  expect(stt.every((entry) => entry.authorization === "Bearer voice-stt-e2e-secret")).toBeTruthy();
  expect(stt.every((entry) => entry.contentType?.startsWith("multipart/form-data"))).toBeTruthy();
  expect(stt.every((entry) => entry.bodyLength > 3_000)).toBeTruthy();
  expect(llm.every((entry) => entry.authorization === "Bearer voice-llm-e2e-secret")).toBeTruthy();
  const ownerTextLlm = llm.find((entry) => entry.bodyText.includes("Текстовый вопрос владельца"));
  const ownerVoiceLlm = llm.find((entry) => entry.bodyText.includes("Привет из браузера"));
  const failedOwnerLlm = llm.find((entry) => entry.bodyText.includes("Спровоцируй отказ провайдера"));
  const recoveredOwnerLlm = llm.find((entry) => entry.bodyText.includes("Восстановление после отказа"));
  const visitorTextLlm = llm.find((entry) => entry.bodyText.includes("Текстовый вопрос visitor"));
  const visitorVoiceLlm = llm.find((entry) => entry.bodyText.includes("Что думает владелец?"));

  expect(ownerTextLlm).toBeDefined();
  expect(ownerVoiceLlm).toBeDefined();
  expect(failedOwnerLlm).toBeDefined();
  expect(recoveredOwnerLlm).toBeDefined();
  expect(visitorTextLlm).toBeDefined();
  expect(visitorVoiceLlm).toBeDefined();

  for (const ownerAnswer of ownerAnswers) {
    expect(ownerTextLlm?.bodyText).toContain(ownerAnswer);
    expect(ownerVoiceLlm?.bodyText).toContain(ownerAnswer);
    expect(failedOwnerLlm?.bodyText).toContain(ownerAnswer);
    expect(recoveredOwnerLlm?.bodyText).toContain(ownerAnswer);
    expect(visitorTextLlm?.bodyText).not.toContain(ownerAnswer);
    expect(visitorVoiceLlm?.bodyText).not.toContain(ownerAnswer);
  }
  expect(visitorTextLlm?.bodyText).toContain("Visitor permissions do not expose owner-reviewed personal context");
  expect(visitorVoiceLlm?.bodyText).toContain("Visitor permissions do not expose owner-reviewed personal context");

  expect(avatar.every((entry) => entry.authorization === "Basic voice-avatar-e2e-secret")).toBeTruthy();
  const spokenText = (streamPath: string): string =>
    avatar
      .filter((entry) => entry.method === "POST" && entry.path.endsWith(streamPath))
      .map((entry) => {
        const payload = JSON.parse(entry.bodyText) as { script?: { input?: string } };
        return payload.script?.input ?? "";
      })
      .filter(Boolean)
      .join(" ");

  expect(spokenText("/stream-1")).toContain("Голосовой ответ владельцу");
  expect(spokenText("/stream-2")).toBe("В visitor scope нет подтверждённых данных владельца");
});
