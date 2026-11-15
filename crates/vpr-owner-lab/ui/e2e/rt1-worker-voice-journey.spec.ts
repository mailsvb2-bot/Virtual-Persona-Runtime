import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

const ownerLabUrl = "http://127.0.0.1:18787";
const providerUrl = "http://127.0.0.1:18790";
const workerUrl = "http://127.0.0.1:18789";
const workerToken = "rt1-local-e2e-secret";

type ProviderRecord = {
  kind: string;
  path: string;
  bodyText: string;
};

type WorkerState = {
  renderer: string;
  video_frames_sent: number;
  pcm_pushes: number;
  active_operations: number;
  audio_senders_live: number;
  video_senders_live: number;
  confirmed_stops: Array<{
    operation_id: string;
    session_id: string;
    audio_stopped: boolean;
    video_stopped: boolean;
  }>;
};

type PeerStats = {
  audioBytes: number;
  videoBytes: number;
  connectionState: string;
};

type WindowWithRt1Peers = Window & { __rt1Peers?: RTCPeerConnection[] };

let ownerToken = "";
let visitorToken = "";

async function installPeerStatsProbe(page: Page) {
  await page.addInitScript(() => {
    const NativePeer = window.RTCPeerConnection;
    const peers: RTCPeerConnection[] = [];
    Object.defineProperty(window, "__rt1Peers", { configurable: true, value: peers });
    Object.defineProperty(window, "RTCPeerConnection", {
      configurable: true,
      writable: true,
      value: class extends NativePeer {
        constructor(configuration?: RTCConfiguration) {
          super(configuration);
          peers.push(this);
        }
      },
    });
  });
}

async function readPeerStats(page: Page): Promise<PeerStats | null> {
  return page.evaluate(async () => {
    const peers = (window as WindowWithRt1Peers).__rt1Peers ?? [];
    const peer = peers[0];
    if (!peer) return null;
    const reports = await peer.getStats();
    let audioBytes = 0;
    let videoBytes = 0;
    reports.forEach((report) => {
      if (report.type !== "inbound-rtp") return;
      const inbound = report as RTCInboundRtpStreamStats & {
        kind?: string;
        mediaType?: string;
      };
      const kind = inbound.kind ?? inbound.mediaType;
      if (kind === "audio") audioBytes += inbound.bytesReceived ?? 0;
      if (kind === "video") videoBytes += inbound.bytesReceived ?? 0;
    });
    return { audioBytes, videoBytes, connectionState: peer.connectionState };
  });
}

async function providerRecords(request: APIRequestContext): Promise<ProviderRecord[]> {
  const response = await request.get(providerUrl + "/__state");
  expect(response.ok()).toBeTruthy();
  return (await response.json()).requests as ProviderRecord[];
}

async function workerState(request: APIRequestContext): Promise<WorkerState> {
  const response = await request.get(workerUrl + "/__state", {
    headers: { Authorization: "Bearer " + workerToken },
  });
  expect(response.ok()).toBeTruthy();
  return (await response.json()) as WorkerState;
}

async function createOwnerAndInvite(page: Page): Promise<{ owner: string; invite: string }> {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.locator("#workflow-state")).toContainText("Ожидает проверки владельцем");
  await page.getByLabel("Идентификатор Persona").fill("rt1-worker-voice-browser-e2e");
  await page.getByRole("button", { name: "Создать Persona" }).click();

  const answers = ["Слушаю внимательно", "Говорю спокойно", "Отвечаю кратко"];
  for (const [index, answer] of answers.entries()) {
    await page.getByPlaceholder("Ответьте своими словами").fill(answer);
    await page.getByRole("button", { name: "Сохранить ответ" }).click();
    await expect(page.locator("#persona-progress")).toContainText(
      String(index + 1) + "/" + String(answers.length),
    );
  }
  await page.getByRole("button", { name: "Перейти к проверке" }).click();
  const accept = page.getByRole("button", { name: "Подтвердить без изменений" });
  await expect(accept).toHaveCount(answers.length);
  for (let index = 0; index < answers.length; index += 1) await accept.first().click();
  await page.getByRole("button", { name: "Подтвердить Persona" }).click();
  await expect(page.locator("#persona-progress")).toContainText("версия 2");

  await page.goto("/rt1", { waitUntil: "domcontentloaded" });
  await expect(page.locator("#status")).toHaveAttribute("data-rt1-ready", "true");
  await page.locator("#provision").click();
  await expect(page.locator("#status")).toContainText("Кабинет создан");
  const owner = await page.locator("#owner-token").inputValue();
  expect(owner.length).toBeGreaterThan(30);
  await page.locator("#persona-version").fill("2");
  await page.locator("#publish").click();
  await expect(page.locator("#invite-token")).not.toHaveValue("");
  const invite = await page.locator("#invite-token").inputValue();
  return { owner, invite };
}

async function joinVisitor(page: Page, invite: string): Promise<string> {
  await expect(page.locator("#status")).toHaveAttribute("data-rt1-ready", "true");
  await page.locator("#visitor-token").fill(invite);
  await page.locator("#consent").check();
  const startPromise = page.waitForResponse((response) =>
    response.url().endsWith("/api/rt1/visitor/start") && response.request().method() === "POST",
  );
  await page.locator("#join").click();
  const startResponse = await startPromise;
  expect(startResponse.status()).toBe(200);
  const start = await startResponse.json();
  expect(start.voice_enabled).toBe(true);
  expect(start.signal.transport.kind).toBe("web_rtc");

  await expect.poll(
    async () => page.locator("#video").evaluate((node) => (node as HTMLVideoElement).videoWidth),
    { timeout: 30_000 },
  ).toBeGreaterThan(0);
  await expect.poll(
    async () => page.locator("#audio").evaluate((node) => {
      const stream = (node as HTMLAudioElement).srcObject as MediaStream | null;
      return stream?.getAudioTracks().length ?? 0;
    }),
    { timeout: 20_000 },
  ).toBe(1);
  await expect.poll(
    async () => (await readPeerStats(page))?.videoBytes ?? 0,
    { timeout: 30_000 },
  ).toBeGreaterThan(0);
  return String(start.session_id);
}

async function submitMicrophone(page: Page) {
  await expect(page.locator("#speak")).toBeEnabled();
  await page.locator("#speak").click();
  await expect(page.locator("#speak")).toHaveText("Отправить речь", { timeout: 15_000 });
  const responsePromise = page.waitForResponse((response) =>
    response.url().endsWith("/api/rt1/visitor/voice") && response.request().method() === "POST",
  );
  await page.locator("#speak").click();
  return responsePromise;
}

async function waitForBothTrackReceipt(request: APIRequestContext, operationId: string) {
  await expect.poll(async () => {
    const state = await workerState(request);
    return state.confirmed_stops.some((receipt) =>
      receipt.operation_id === operationId && receipt.audio_stopped && receipt.video_stopped,
    );
  }, { timeout: 20_000 }).toBe(true);
  const state = await workerState(request);
  expect(state.active_operations).toBe(0);
  const receipt = state.confirmed_stops.find((item) => item.operation_id === operationId);
  expect(receipt).toMatchObject({ audio_stopped: true, video_stopped: true });
}

test.describe.serial("RT1 verified worker visitor voice journey (synthetic local fixtures; not D-ID)", () => {
  test("browser microphone uses STT/LLM/TTS and worker RTP, then interrupt confirms both senders", async ({ page, request }) => {
    await installPeerStatsProbe(page);
    await page.context().grantPermissions(["microphone"], { origin: ownerLabUrl });
    const credentials = await createOwnerAndInvite(page);
    ownerToken = credentials.owner;
    visitorToken = credentials.invite;

    const operationId = await joinVisitor(page, visitorToken);
    await page.locator("#visitor-text").fill("Текстовый вопрос visitor");
    const textPromise = page.waitForResponse((response) =>
      response.url().endsWith("/api/rt1/visitor/text") && response.request().method() === "POST",
    );
    await page.locator("#send-text").click();
    const textResponse = await textPromise;
    expect(textResponse.status()).toBe(200);
    expect((await textResponse.json()).reply).toContain("Текстовый ответ visitor");
    await expect(page.locator("#speech-result")).toContainText("Текстовый ответ visitor");

    const voiceResponse = await submitMicrophone(page);
    expect(voiceResponse.status()).toBe(200);
    const voice = await voiceResponse.json();
    expect(voice.transcript).toBe("Привет из браузера");
    expect(voice.reply).toContain("Третья фраза");
    await expect(page.locator("#speech-result")).toContainText("Привет из браузера");
    await expect(page.locator("#speech-result")).toContainText("Третья фраза");

    const providers = await providerRecords(request);
    expect(providers.some((item) => item.kind === "stt")).toBe(true);
    expect(providers.some((item) => item.kind === "llm" && item.bodyText.includes("Текстовый вопрос visitor"))).toBe(true);
    expect(providers.filter((item) => item.kind === "tts").length).toBeGreaterThanOrEqual(2);
    const activeWorker = await workerState(request);
    expect(activeWorker.renderer).toContain("synthetic-generated-face-test-only");
    expect(activeWorker.video_frames_sent).toBeGreaterThan(0);
    expect(activeWorker.pcm_pushes).toBeGreaterThanOrEqual(2);
    await expect.poll(
      async () => (await readPeerStats(page))?.audioBytes ?? 0,
      { timeout: 20_000 },
    ).toBeGreaterThan(0);

    const ttsBeforeInterrupt = providers.filter((item) => item.kind === "tts").length;
    const pcmBeforeInterrupt = activeWorker.pcm_pushes;
    const llmBeforeInterrupt = providers.filter((item) => item.kind === "llm").length;
    const interruptedVoice = submitMicrophone(page);
    await expect.poll(
      async () => (await providerRecords(request)).filter((item) => item.kind === "llm").length,
      { timeout: 20_000 },
    ).toBe(llmBeforeInterrupt + 1);

    const interruptPromise = page.waitForResponse((response) =>
      response.url().endsWith("/api/rt1/visitor/interrupt") && response.request().method() === "POST",
    );
    const closePromise = page.waitForResponse(
      (response) => response.url().endsWith("/api/rt1/visitor/close") && response.request().method() === "POST",
      { timeout: 30_000 },
    ).catch(() => null);
    await expect(page.locator("#interrupt")).toBeEnabled();
    await page.locator("#interrupt").click();
    const interruptResponse = await interruptPromise;
    expect(interruptResponse.status()).toBe(200);
    expect((await interruptResponse.json()).remote_video_stop_confirmed).toBe(true);
    await expect(page.locator("#speak")).toBeDisabled();

    const interruptedResponse = await interruptedVoice;
    expect(interruptedResponse.status()).toBe(409);
    if (await page.locator("#leave").isEnabled()) {
      await page.locator("#leave").click({ timeout: 2_000 }).catch(() => {});
    }
    const closeResponse = await closePromise;
    if (!closeResponse) throw new Error("visitor close was not completed after confirmed interrupt");
    expect(closeResponse.status()).toBe(200);
    expect((await closeResponse.json()).remote_stop_confirmed).toBe(true);

    await page.waitForTimeout(4_500);
    const providersAfterInterrupt = await providerRecords(request);
    const workerAfterInterrupt = await workerState(request);
    expect(providersAfterInterrupt.filter((item) => item.kind === "tts").length).toBe(ttsBeforeInterrupt);
    expect(workerAfterInterrupt.pcm_pushes).toBe(pcmBeforeInterrupt);
    await waitForBothTrackReceipt(request, operationId);
  });

  test("owner pause revokes an in-flight visitor stream before delayed LLM tail reaches TTS/RTP", async ({ page, request }) => {
    await installPeerStatsProbe(page);
    await page.context().grantPermissions(["microphone"], { origin: ownerLabUrl });
    await page.goto("/rt1", { waitUntil: "domcontentloaded" });
    await expect(page.locator("#status")).toHaveAttribute("data-rt1-ready", "true");
    await page.locator("#owner-token").fill(ownerToken);
    const operationId = await joinVisitor(page, visitorToken);

    const providersBefore = await providerRecords(request);
    const llmBefore = providersBefore.filter((item) => item.kind === "llm").length;
    const ttsBefore = providersBefore.filter((item) => item.kind === "tts").length;
    const workerBefore = await workerState(request);
    const pcmBefore = workerBefore.pcm_pushes;
    const inFlightVoice = submitMicrophone(page);
    await expect.poll(
      async () => (await providerRecords(request)).filter((item) => item.kind === "llm").length,
      { timeout: 20_000 },
    ).toBe(llmBefore + 1);

    const pausePromise = page.waitForResponse((response) =>
      response.url().endsWith("/api/rt1/owner/pause") && response.request().method() === "POST",
    );
    const closePromise = page.waitForResponse(
      (response) => response.url().endsWith("/api/rt1/visitor/close") && response.request().method() === "POST",
      { timeout: 30_000 },
    );
    await page.locator("#pause").click();
    const pauseResponse = await pausePromise;
    expect(pauseResponse.status()).toBe(200);
    const closeResponse = await closePromise;
    expect(closeResponse.status()).toBe(200);
    expect((await closeResponse.json()).remote_stop_confirmed).toBe(true);

    const voiceResponse = await inFlightVoice;
    expect(voiceResponse.status()).toBe(409);
    await page.waitForTimeout(4_500);
    const providersAfter = await providerRecords(request);
    const workerAfter = await workerState(request);
    expect(providersAfter.filter((item) => item.kind === "tts").length).toBe(ttsBefore);
    expect(workerAfter.pcm_pushes).toBe(pcmBefore);
    await waitForBothTrackReceipt(request, operationId);
  });
});
