import {
  expect,
  test,
  type APIRequestContext,
  type Page,
} from "@playwright/test";

import { installProviderAutoConnect } from "./provider-bootstrap.js";

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
    }).__vprLiveKitCommands?.filter((command) => command.topic === "did.speak").length ?? 0,
  )).toBe(1);

  const spoken = await page.evaluate(() => (
    (window as typeof window & {
      __vprLiveKitCommands?: Array<{ topic: string; text: string }>;
    }).__vprLiveKitCommands?.find((command) => command.topic === "did.speak")?.text ?? ""
  ));
  expect(spoken).toContain(reply);

  await expect(page.locator("#status")).toContainText(`Вы: ${transcript}`);
  await expect(page.locator("#status")).toContainText(`Ответ: ${reply}`);

  // Do not end fake provider playback until the browser has actually observed remote
  // audio and the backend has accepted canonical audio_started evidence. Otherwise
  // a fast playback_done can race requestAnimationFrame-based audio observation and
  // make the same candidate pass or fail depending on runner scheduling.
  await expect(page.locator("#readiness-voice")).toHaveText("Готов");

  await page.evaluate(() => {
    const fakeWindow = window as typeof window & { __vprExpressivePlaybackDone?: () => void };
    fakeWindow.__vprExpressivePlaybackDone?.();
  });

  await expect.poll(async () => page.evaluate(
    () => (window as typeof window & {
      __vprLiveKitCommands?: Array<{ topic: string; text: string }>;
    }).__vprLiveKitCommands?.filter((command) => command.topic === "did.speak").length ?? 0,
  )).toBe(1);
};

test("Expressive LiveKit voice path reaches canonical playback, A/V sync and recovery", async ({
  page,
  request,
}) => {
  const bootstrap = await request.get(`${ownerLabUrl}/api/bootstrap`);
  expect(bootstrap.ok()).toBeTruthy();
  const csrf = String((await bootstrap.json()).csrf_token);
  await setupReviewedPersona(request, csrf);

  await page.addInitScript({ path: "e2e/fake-livekit-client.js" });
  await installProviderAutoConnect(page);
  await page.route("**/api/evidence/media", async (route) => {
    const request = route.request();
    if (request.method() === "POST") {
      const body = request.postDataJSON() as { kind?: string } | null;
      if (body?.kind === "audio_started") {
        await new Promise<void>((resolve) => setTimeout(resolve, 300));
      }
    }
    await route.continue();
  });
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-vpr-provider-auto-connect", "clicked");
  await expect(page.locator("#persona-progress")).toContainText("версия 2");
  await expect(page.locator("#status")).toContainText("LiveKit согласован");
  await expect(page.locator(".stage")).toHaveClass(/has-video/);
  await expect(page.locator("#readiness-text")).toHaveText("Готов");
  await expect(page.locator("#readiness-video")).toHaveText("Готов");
  await expect(page.locator("#readiness-voice")).toHaveText("Подготовка…");

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

  await page.evaluate(() => {
    const fakeWindow = window as typeof window & { __vprExpressiveLoseAudio?: () => void };
    fakeWindow.__vprExpressiveLoseAudio?.();
  });
  await expect(voiceButton).toBeEnabled();
  await expect(page.locator("#speak")).toBeEnabled();
  await expect(page.locator("#status")).toContainText(
    "Аудиопоток аватара потерян. Микрофон и текст остаются доступны",
  );

  await page.evaluate(() => {
    const fakeWindow = window as typeof window & { __vprExpressiveRestoreAudio?: () => void };
    fakeWindow.__vprExpressiveRestoreAudio?.();
  });
  await expect(voiceButton).toBeEnabled();

  await recordStreamingVoiceTurn(
    page,
    "Привет из браузера",
    "Сначала уточню один важный момент, затем продолжу. Третья фраза.",
  );
  await expect(page.locator("#readiness-voice")).toHaveText("Готов");
  await expect(page.locator("#readiness-video")).toHaveText("Готов");

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
  }, { timeout: 10_000 }).toBeTruthy();

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
  const speak = commands.filter((command) => command.topic === "did.speak");
  expect(speak).toHaveLength(1);
  expect(JSON.parse(speak[0]?.text ?? "{}").script.input).toBe(
    "Сначала уточню один важный момент, затем продолжу. Третья фраза.",
  );

  // Start a second voice turn and interrupt it while the LLM tail is still open.
  // Client-text avatars receive only a complete generated reply, so the interrupted turn must
  // never emit even a partial did.speak command.
  await voiceButton.click();
  await expect(voiceButton).toHaveText("Остановить и отправить");
  await voiceButton.click();
  await expect.poll(async () => page.evaluate(
    () => (window as typeof window & {
      __vprLiveKitCommands?: Array<{ topic: string; text: string }>;
    }).__vprLiveKitCommands?.filter((command) => command.topic === "did.speak").length ?? 0,
  )).toBe(1);

  const interrupt = page.getByRole("button", { name: "Прервать", exact: true });
  await expect(interrupt).toBeEnabled();
  await interrupt.click();
  await expect(page.locator("#status")).toContainText("TURN_CANCELLED");

  await page.waitForTimeout(800);
  const commandsAfterInterrupt = await page.evaluate(
    () => (window as typeof window & {
      __vprLiveKitCommands?: Array<{ topic: string; text: string }>;
    }).__vprLiveKitCommands ?? [],
  );
  expect(commandsAfterInterrupt.filter((command) => command.topic === "did.speak")).toHaveLength(1);
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

  expect(sttRequests).toHaveLength(2);
  expect(sttRequests[0]?.method).toBe("WEBSOCKET");
  expect(sttRequests[0]?.path).toBe("/v1/listen");
  expect(sttRequests[0]?.query).toContain("model=nova-3");
  expect(sttRequests[0]?.query).toContain("encoding=linear16");
  expect(sttRequests[0]?.query).toContain("sample_rate=16000");
  expect(sttRequests[0]?.query).toContain("channels=1");
  expect(sttRequests[0]?.query).toContain("interim_results=true");
  expect(sttRequests[0]?.query).toContain("smart_format=true");
  expect(sttRequests[0]?.query).toContain("language=ru");
  expect(sttRequests[0]?.authorization).toBe("Token expressive-stt-e2e-secret");
  expect(sttRequests[0]?.contentType).toBeNull();

  expect(llmRequests).toHaveLength(2);
  expect(llmRequests[0]?.authorization).toBe("Bearer expressive-llm-e2e-secret");
  expect(llmRequests[0]?.bodyText).toContain('"model":"deepseek-flash"');
  expect(llmRequests[0]?.bodyText).toContain('"reasoning_effort":"none"');
  expect(llmRequests[0]?.bodyText).toContain('"thinking":{"type":"disabled"}');
  expect(llmRequests[0]?.bodyText).toContain('"max_tokens":96');
  expect(llmRequests[1]?.authorization).toBe("Bearer expressive-llm-e2e-secret");
  expect(llmRequests[1]?.bodyText).toContain("Что думает владелец?");

  expect(avatarRequests.some((entry) =>
    entry.method === "GET" && entry.path === "/agents/voice-e2e-expressive-agent"
  )).toBeTruthy();
  expect(avatarRequests.some((entry) =>
    entry.method === "POST"
      && entry.path === "/v2/agents/voice-e2e-expressive-agent/sessions"
  )).toBeTruthy();
  expect(avatarRequests.some((entry) => entry.path.includes("/streams"))).toBeFalsy();
});
