import {
  expect,
  test,
  type APIRequestContext,
  type Page,
} from "@playwright/test";

const ownerLabUrl = "http://127.0.0.1:18791";
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
    { persona_id: "owner-expressive-e2e" },
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

const installExpressiveBrowserFakes = async (page: Page): Promise<void> => {
  await page.addInitScript(() => {
    let remoteSpeech = false;
    let trackSequence = 0;
    const commands: Array<{ topic: string; text: string }> = [];
    type EventHandler = (...args: unknown[]) => void;

    class FakeTrack {
      id: string;
      constructor(readonly kind: "audio" | "video") {
        trackSequence += 1;
        this.id = `expressive-track-${trackSequence}`;
      }
      stop(): void {}
    }

    class FakeMediaStream {
      private readonly tracks: FakeTrack[];
      constructor(tracks: FakeTrack[] = []) {
        this.tracks = [...tracks];
      }
      getTracks(): FakeTrack[] {
        return [...this.tracks];
      }
      addTrack(track: FakeTrack): void {
        this.tracks.push(track);
      }
    }

    Object.defineProperty(window, "MediaStream", {
      configurable: true,
      value: FakeMediaStream,
    });
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: {
        getUserMedia: async () => new FakeMediaStream([new FakeTrack("audio")]),
      },
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
      createAnalyser(): FakeAnalyser {
        return new FakeAnalyser();
      }
      createGain(): FakeGain {
        return new FakeGain();
      }
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

    const roomEvents = {
      TrackSubscribed: "track-subscribed",
      TrackUnsubscribed: "track-unsubscribed",
      DataReceived: "data-received",
      Reconnecting: "reconnecting",
      Reconnected: "reconnected",
      Disconnected: "disconnected",
    };

    class FakeRemoteTrack {
      readonly mediaStreamTrack: FakeTrack;
      constructor(readonly kind: "audio" | "video") {
        this.mediaStreamTrack = new FakeTrack(kind);
      }
      attach(element: HTMLMediaElement): HTMLMediaElement {
        element.style.width = "4096px";
        element.style.height = "4096px";
        return element;
      }
      async getRTCStatsReport(): Promise<RTCStatsReport> {
        const timestamp = this.kind === "audio" ? 1_000 : 1_060;
        return new Map([
          [
            `${this.kind}-inbound`,
            {
              type: "inbound-rtp",
              kind: this.kind,
              packetsReceived: 20,
              estimatedPlayoutTimestamp: timestamp,
            },
          ],
        ]) as unknown as RTCStatsReport;
      }
    }

    class FakeRoom {
      readonly localParticipant = {
        sendText: async (text: string, options: { topic: string }): Promise<void> => {
          commands.push({ topic: options.topic, text });
          if (options.topic === "did.speak") remoteSpeech = true;
          if (options.topic === "did.interrupt") remoteSpeech = false;
        },
      };
      private readonly handlers = new Map<string, EventHandler[]>();
      private videoTrack: FakeRemoteTrack | null = null;

      constructor() {
        const fakeWindow = window as typeof window & {
          __vprExpressiveDisconnect?: () => void;
          __vprExpressiveLoseVideo?: () => void;
          __vprExpressiveRestoreVideo?: () => void;
          __vprExpressivePlaybackDone?: () => void;
        };
        fakeWindow.__vprExpressiveDisconnect = () => {
          for (const handler of this.handlers.get(roomEvents.Disconnected) ?? []) handler();
        };
        fakeWindow.__vprExpressiveLoseVideo = () => {
          const track = this.videoTrack;
          if (!track) return;
          this.videoTrack = null;
          this.emit(roomEvents.TrackUnsubscribed, track);
        };
        fakeWindow.__vprExpressiveRestoreVideo = () => {
          if (this.videoTrack) return;
          const track = new FakeRemoteTrack("video");
          this.videoTrack = track;
          this.emit(roomEvents.TrackSubscribed, track);
        };
        fakeWindow.__vprExpressivePlaybackDone = () => {
          remoteSpeech = false;
          this.emit(
            roomEvents.DataReceived,
            new TextEncoder().encode(JSON.stringify({ subject: "stream-video/done" })),
          );
        };
      }

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
        this.videoTrack = new FakeRemoteTrack("video");
        this.emit(roomEvents.TrackSubscribed, this.videoTrack);
        this.emit(roomEvents.TrackSubscribed, new FakeRemoteTrack("audio"));
      }

      async disconnect(): Promise<void> {}
    }

    const fakeWindow = window as typeof window & {
      LivekitClient?: unknown;
      __vprLiveKitCommands?: Array<{ topic: string; text: string }>;
    };
    fakeWindow.__vprLiveKitCommands = commands;
    fakeWindow.LivekitClient = {
      Room: FakeRoom,
      RoomEvent: roomEvents,
    };

    Object.defineProperty(HTMLVideoElement.prototype, "requestVideoFrameCallback", {
      configurable: true,
      value(callback: () => void): number {
        queueMicrotask(callback);
        return 1;
      },
    });
    Object.defineProperty(window, "AudioContext", {
      configurable: true,
      value: FakeAudioContext,
    });
    Object.defineProperty(window, "AudioWorkletNode", {
      configurable: true,
      value: FakeAudioWorkletNode,
    });
  });
};

const recordStreamingVoiceTurn = async (
  page: Page,
  transcript: string,
  reply: string,
): Promise<void> => {
  const voice = page.locator("#voice");
  await voice.click();
  await expect(voice).toHaveText("Остановить и отправить");
  await voice.click();

  await expect.poll(async () => page.evaluate(
    () => (window as typeof window & {
      __vprLiveKitCommands?: Array<{ topic: string; text: string }>;
    }).__vprLiveKitCommands?.some((command) => command.topic === "did.speak") ?? false,
  )).toBeTruthy();

  // The fixture keeps the LLM stream open after its first complete phrase. Seeing did.speak before
  // the final status proves browser delivery no longer waits for full generation.
  await expect(page.locator("#status")).not.toContainText(`Вы: ${transcript}`);
  await expect(page.locator("#status")).toContainText(`Вы: ${transcript}`);
  await expect(page.locator("#status")).toContainText(`Ответ: ${reply}`);
};

test("Expressive LiveKit voice path reaches canonical playback, A/V sync and recovery", async ({
  page,
  request,
}) => {
  const bootstrap = await request.get(`${ownerLabUrl}/api/bootstrap`);
  expect(bootstrap.ok()).toBeTruthy();
  const csrf = String((await bootstrap.json()).csrf_token);
  await setupReviewedPersona(request, csrf);

  await installExpressiveBrowserFakes(page);
  await page.goto("/");
  await page.getByRole("checkbox").check();
  await page.getByRole("button", { name: "Подключить аватар" }).click();
  await expect(page.locator("#status")).toContainText("LiveKit согласован");
  await expect(page.locator(".stage")).toHaveClass(/has-video/);

  const layout = await page.evaluate(() => {
    const stage = document.querySelector<HTMLElement>(".stage");
    const avatar = document.querySelector<HTMLVideoElement>("#avatar");
    if (!stage || !avatar) throw new Error("missing stage");
    const stageRect = stage.getBoundingClientRect();
    const avatarRect = avatar.getBoundingClientRect();
    return {
      overflow: document.documentElement.scrollWidth > window.innerWidth,
      objectFit: getComputedStyle(avatar).objectFit,
      stageWidth: stageRect.width,
      stageHeight: stageRect.height,
      avatarWidth: avatarRect.width,
      avatarHeight: avatarRect.height,
    };
  });
  expect(layout.overflow).toBe(false);
  expect(layout.objectFit).toBe("contain");
  expect(layout.avatarWidth).toBeLessThanOrEqual(layout.stageWidth);
  expect(layout.avatarHeight).toBeLessThanOrEqual(layout.stageHeight);

  const voiceButton = page.locator("#voice");
  await expect(voiceButton).toBeEnabled();
  await page.evaluate(() => {
    const fakeWindow = window as typeof window & { __vprExpressiveLoseVideo?: () => void };
    fakeWindow.__vprExpressiveLoseVideo?.();
  });
  await expect(page.locator(".stage")).not.toHaveClass(/has-video/);
  await expect(voiceButton).toBeEnabled();
  await expect(page.locator("#status")).toContainText(
    "Видео-поток аватара потерян. Голос остаётся доступен",
  );

  await page.evaluate(() => {
    const fakeWindow = window as typeof window & { __vprExpressiveRestoreVideo?: () => void };
    fakeWindow.__vprExpressiveRestoreVideo?.();
  });
  await expect(page.locator(".stage")).toHaveClass(/has-video/);
  await expect(voiceButton).toBeEnabled();

  await recordStreamingVoiceTurn(
    page,
    "Привет из браузера",
    "Голосовой ответ владельцу.",
  );

  await expect.poll(async () => {
    const evidence = await request.get(`${ownerLabUrl}/api/evidence/session`);
    const snapshot = await evidence.json() as {
      canonical_playback_proven: boolean;
      av_sync_proven: boolean;
    };
    return snapshot.canonical_playback_proven && snapshot.av_sync_proven;
  }).toBeTruthy();

  const evidence = await request.get(`${ownerLabUrl}/api/evidence/session`);
  const snapshot = await evidence.json() as {
    canonical_playback_proven: boolean;
    av_sync_proven: boolean;
    av_sync_samples: Array<{
      sample_sequence: number;
      reference: string;
      absolute_offset_millis: number;
    }>;
    media_events: Array<{ kind: string }>;
  };
  expect(snapshot.canonical_playback_proven).toBeTruthy();
  expect(snapshot.av_sync_proven).toBeTruthy();
  expect(snapshot.av_sync_samples).toHaveLength(3);
  expect(snapshot.av_sync_samples.map((sample) => sample.sample_sequence)).toEqual([1, 2, 3]);
  expect(snapshot.av_sync_samples.every((sample) =>
    sample.reference === "web_rtc_estimated_playout_timestamp"
      && sample.absolute_offset_millis === 60
  )).toBeTruthy();

  await expect(page.locator("#metric-stt")).toHaveText(/\d+ мс/);
  await expect(page.locator("#metric-llm")).toHaveText(/\d+ мс/);
  await expect(page.locator("#metric-llm-first")).toHaveText(/\d+ мс/);
  await expect(page.locator("#metric-server-total")).toHaveText(/\d+ мс/);
  await expect(page.locator("#metric-first-audio")).toHaveText(/\d+ мс/);
  await expect(page.locator("#metric-video-ready")).toHaveText(/\d+ мс/);
  await expect(page.locator("#metric-av-sync")).toHaveText("60 мс · 3 изм.");
  await expect(page.locator("#metric-playback")).toHaveText("подтверждён");
  await expect(page.locator("#metric-cost")).toHaveText("провайдер не сообщил стоимость");

  const commands = await page.evaluate(
    () => (window as typeof window & {
      __vprLiveKitCommands?: Array<{ topic: string; text: string }>;
    }).__vprLiveKitCommands ?? [],
  );
  const speak = commands.find((command) => command.topic === "did.speak");
  expect(speak).toBeDefined();
  expect(JSON.parse(speak?.text ?? "{}")).toMatchObject({
    script: { type: "text", input: "Голосовой ответ владельцу." },
  });

  const interrupt = page.getByRole("button", { name: "Прервать", exact: true });
  await expect(interrupt).toBeEnabled();
  await interrupt.click();
  await expect.poll(async () => {
    const current = await request.get(`${ownerLabUrl}/api/evidence/session`);
    const currentSnapshot = await current.json() as {
      media_events: Array<{ kind: string }>;
    };
    return currentSnapshot.media_events.some((event) => event.kind === "interruption_stopped");
  }).toBeTruthy();

  const commandsAfterInterrupt = await page.evaluate(
    () => (window as typeof window & {
      __vprLiveKitCommands?: Array<{ topic: string; text: string }>;
    }).__vprLiveKitCommands ?? [],
  );
  expect(commandsAfterInterrupt.some((command) => command.topic === "did.interrupt")).toBeTruthy();

  await page.evaluate(() => {
    const fakeWindow = window as typeof window & { __vprExpressiveDisconnect?: () => void };
    fakeWindow.__vprExpressiveDisconnect?.();
  });
  await expect(page.locator("#status")).toContainText("Сессия закрыта");

  const providerState = await request.get(`${providerUrl}/__state`);
  expect(providerState.ok()).toBeTruthy();
  const { requests } = await providerState.json() as {
    requests: Array<{
      kind: "stt" | "llm" | "avatar";
      method: string;
      path: string;
      query: string;
      authorization: string | null;
      contentType: string | null;
      bodyText: string;
    }>;
  };
  const sttRequests = requests.filter((entry) => entry.kind === "stt");
  const llmRequests = requests.filter((entry) => entry.kind === "llm");
  const avatarRequests = requests.filter((entry) => entry.kind === "avatar");

  expect(sttRequests).toHaveLength(1);
  expect(sttRequests[0]?.path).toBe("/v1/listen");
  expect(sttRequests[0]?.query).toContain("model=nova-3");
  expect(sttRequests[0]?.query).toContain("encoding=linear16");
  expect(sttRequests[0]?.query).toContain("sample_rate=16000");
  expect(sttRequests[0]?.query).toContain("channels=1");
  expect(sttRequests[0]?.query).toContain("smart_format=true");
  expect(sttRequests[0]?.query).toContain("language=ru");
  expect(sttRequests[0]?.authorization).toBe("Token expressive-stt-e2e-secret");
  expect(sttRequests[0]?.contentType).toBe("application/octet-stream");

  expect(llmRequests).toHaveLength(1);
  expect(llmRequests[0]?.authorization).toBe("Bearer expressive-llm-e2e-secret");
  expect(llmRequests[0]?.bodyText).toContain('"model":"deepseek-flash"');
  expect(llmRequests[0]?.bodyText).toContain('"reasoning_effort":"none"');
  expect(llmRequests[0]?.bodyText).toContain('"thinking":{"type":"disabled"}');
  expect(llmRequests[0]?.bodyText).toContain('"max_tokens":96');

  expect(avatarRequests.some((entry) =>
    entry.method === "GET" && entry.path === "/agents/voice-e2e-expressive-agent"
  )).toBeTruthy();
  expect(avatarRequests.some((entry) =>
    entry.method === "POST"
      && entry.path === "/v2/agents/voice-e2e-expressive-agent/sessions"
  )).toBeTruthy();
  expect(avatarRequests.some((entry) => entry.path.includes("/streams"))).toBeFalsy();
});
