import {
  expect,
  test,
  type APIRequestContext,
} from "@playwright/test";

import { installProviderAutoConnect } from "./provider-bootstrap.js";

const ownerLabUrl = "http://127.0.0.1:18791";
const providerUrl = "http://127.0.0.1:18790";
const mailboxUrl = "http://127.0.0.1:18792/__journey/report/expressive";
const EXPRESSIVE_JOURNEY_COMPLETION_TIMEOUT_MS = 90_000;
const EXPRESSIVE_JOURNEY_TEST_TIMEOUT_MS = 150_000;
const ownerAnswers = [
  "Я создаю виртуальных персонажей",
  "Отвечай кратко и спокойно",
  "Точность важнее уверенного выдумывания",
];

type ExpressiveJourneyReport = {
  status: "ok" | "failed";
  error?: string;
  layout?: {
    overflow: boolean;
    objectFit: string;
    stageWidth: number;
    stageHeight: number;
    avatarWidth: number;
    avatarHeight: number;
  };
  evidence?: {
    canonical_playback_proven: boolean;
    av_sync_proven: boolean;
    av_sync_samples: Array<{
      sample_sequence: number;
      reference: string;
      absolute_offset_millis: number;
    }>;
    media_events: Array<{ kind: string }>;
    voice_attempts: Array<{
      status: string;
      canonical_playback_confirmed: boolean;
    }>;
  };
  metrics?: {
    stt: string;
    llm: string;
    llmFirst: string;
    serverTotal: string;
    backendComplete: string;
    clientDelivery: string;
    providerAudioDelay: string;
    firstAudio: string;
    videoReady: string;
    avSync: string;
    playback: string;
    cost: string;
  };
  commands?: Array<{ topic: string; text: string }>;
};

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

test("Expressive LiveKit generation-only events never grant canonical playback", async ({
  page,
  request,
}) => {
  test.setTimeout(EXPRESSIVE_JOURNEY_TEST_TIMEOUT_MS);

  const bootstrap = await request.get(`${ownerLabUrl}/api/bootstrap`);
  expect(bootstrap.ok()).toBeTruthy();
  const csrf = String((await bootstrap.json()).csrf_token);
  await setupReviewedPersona(request, csrf);

  let report: ExpressiveJourneyReport | null = null;
  let lastJourneyPhase = "not-started";
  const resetMailbox = await request.delete(mailboxUrl);
  expect(resetMailbox.ok()).toBeTruthy();

  const readJourney = async (): Promise<void> => {
    const response = await request.get(mailboxUrl);
    if (!response.ok()) return;
    const payload = await response.json() as {
      events?: Array<ExpressiveJourneyReport | { kind: "phase"; phase: string }>;
    };
    report = null;
    lastJourneyPhase = "not-started";
    for (const event of Array.isArray(payload.events) ? payload.events : []) {
      if ("kind" in event && event.kind === "phase") {
        lastJourneyPhase = event.phase;
      } else {
        report = event as ExpressiveJourneyReport;
      }
    }
  };

  await page.addInitScript({ path: "e2e/fake-livekit-client.js" });
  await installProviderAutoConnect(page);
  await page.addInitScript({ path: "e2e/expressive-journey-driver.js" });
  let failedGenerationEvidenceOnce = false;
  await page.route("**/api/evidence/media", async (route) => {
    const incoming = route.request();
    if (incoming.method() === "POST") {
      const body = incoming.postDataJSON() as { kind?: string } | null;
      if (body?.kind === "provider_video_generation_done" && !failedGenerationEvidenceOnce) {
        failedGenerationEvidenceOnce = true;
        await route.fulfill({
          status: 503,
          contentType: "application/json",
          body: '{"ok":false,"code":"PROVIDER_EVIDENCE_TEMPORARY_FAILURE"}',
        });
        return;
      }
      if (body?.kind === "audio_started") {
        await new Promise<void>((resolve) => setTimeout(resolve, 300));
      }
    }
    await route.continue();
  });
  let authorizedReplayPreparations = 0;
  let rejectedReplayOnce = false;
  await page.route("**/api/avatar/resume-answer", async (route) => {
    if (route.request().method() === "POST") {
      authorizedReplayPreparations += 1;
      if (!rejectedReplayOnce) {
        rejectedReplayOnce = true;
        await route.fulfill({
          status: 409,
          contentType: "application/json",
          body: '{"ok":false,"code":"INVALID_STATE_TRANSITION"}',
        });
        return;
      }
    }
    await route.continue();
  });
  await page.goto("/");

  await expect.poll(async () => {
    await readJourney();
    if (!report) return `pending:${lastJourneyPhase}`;
    return report.status === "failed"
      ? `failed:${report.error ?? "unknown"}@phase:${lastJourneyPhase}`
      : report.status;
  }, {
    timeout: EXPRESSIVE_JOURNEY_COMPLETION_TIMEOUT_MS,
    intervals: [100, 250, 500],
    message: "Expressive journey must publish a terminal controller-independent report",
  }).toBe("ok");

  await readJourney();
  expect(report).not.toBeNull();
  expect(report?.status).toBe("ok");

  const layout = report?.layout;
  expect(layout).toBeDefined();
  expect(layout?.overflow).toBe(false);
  expect(layout?.objectFit).toBe("contain");
  expect(layout?.avatarWidth ?? Number.POSITIVE_INFINITY).toBeLessThanOrEqual(
    layout?.stageWidth ?? 0,
  );
  expect(layout?.avatarHeight ?? Number.POSITIVE_INFINITY).toBeLessThanOrEqual(
    layout?.stageHeight ?? 0,
  );

  const evidence = report?.evidence;
  expect(evidence).toBeDefined();
  expect(evidence?.canonical_playback_proven).toBe(false);
  expect(evidence?.av_sync_proven).toBe(false);
  expect(evidence?.av_sync_samples).toHaveLength(3);
  expect(evidence?.av_sync_samples.map((sample) => sample.sample_sequence)).toEqual([1, 2, 3]);
  expect(evidence?.av_sync_samples.every((sample) =>
    sample.reference === "web_rtc_estimated_playout_timestamp"
      && sample.absolute_offset_millis === 60
  )).toBeTruthy();
  expect(evidence?.voice_attempts.some((attempt) =>
    attempt.status === "completed" && !attempt.canonical_playback_confirmed
  )).toBeTruthy();
  expect(evidence?.media_events.some((event) => event.kind === "backend_complete_received")).toBeTruthy();
  expect(evidence?.media_events.some((event) => event.kind === "client_delivery_sent")).toBeTruthy();
  expect(evidence?.media_events.some((event) => event.kind === "provider_data_received")).toBeTruthy();
  expect(evidence?.media_events.some(
    (event) => event.kind === "provider_video_generation_done",
  )).toBeTruthy();
  expect(failedGenerationEvidenceOnce).toBe(true);
  expect(evidence?.media_events.some(
    (event) => event.kind === "provider_video_generation_started",
  )).toBeTruthy();
  expect(evidence?.media_events.some(
    (event) => event.kind === "provider_informational_event",
  )).toBeTruthy();
  for (const category of [
    "provider_unknown_chat_event",
    "provider_unknown_video_event",
    "provider_unknown_tool_event",
    "provider_unknown_other_event",
  ]) {
    expect(evidence?.media_events.some((event) => event.kind === category)).toBeTruthy();
  }
  // D-ID's arbitrary subject and payload never become evidence text.
  expect(JSON.stringify(evidence)).not.toContain("PRIVATE_MEDIA_DIAGNOSTIC_MUST_NOT_LEAK");
  expect(JSON.stringify(evidence)).not.toContain("new-private-event");
  expect(evidence?.media_events.some(
    (event) => event.kind === "provider_playback_done_received",
  )).toBe(false);
  expect(evidence?.media_events.some(
    (event) => event.kind === "provider_event_parse_failed",
  )).toBeFalsy();
  expect(evidence?.media_events.some((event) => event.kind === "playback_completed")).toBe(false);
  // Recovery is strict-RT0-only; this fixture intentionally does not enable that toggle.
  expect(evidence?.media_events.some(
    (event) => event.kind === "playback_recovery_triggered",
  )).toBe(false);

  const metrics = report?.metrics;
  expect(metrics).toBeDefined();
  expect(metrics?.stt).toMatch(/\d+ мс/);
  expect(metrics?.llm).toMatch(/\d+ мс/);
  expect(metrics?.llmFirst).toMatch(/\d+ мс/);
  expect(metrics?.serverTotal).toMatch(/\d+ мс/);
  expect(metrics?.backendComplete).toMatch(/\d+ мс/);
  expect(metrics?.clientDelivery).toMatch(/\d+ мс/);
  expect(metrics?.providerAudioDelay).toMatch(/\d+ мс/);
  expect(metrics?.firstAudio).toMatch(/\d+ мс/);
  expect(metrics?.videoReady).toMatch(/\d+ мс/);
  expect(metrics?.avSync).toBe("ещё не доказан");
  expect(metrics?.playback).toBe("ожидание");
  expect(metrics?.cost).toBe("провайдер не сообщил стоимость");

  const commands = report?.commands ?? [];
  const speak = commands.filter((command) => command.topic === "did.speak");
  expect(rejectedReplayOnce).toBe(true);
  expect(authorizedReplayPreparations).toBe(2);
  expect(speak.length).toBeGreaterThanOrEqual(3);
  const replayed = JSON.parse(speak.at(-1)?.text ?? "{}");
  expect(replayed.script?.input).toBe("Третья фраза.");
  expect(replayed.script?.should_queue_speaks).toBe(true);
  const streamedReply = speak.slice(0, -1).map((command) => {
    const payload = JSON.parse(command.text ?? "{}");
    expect(payload.script?.should_queue_speaks).toBe(true);
    return String(payload.script?.input ?? "");
  }).join(" ").replace(/\s+/g, " ").trim();
  expect(streamedReply).toBe(
    "Сначала уточню один важный момент, затем продолжу. Третья фраза.",
  );
  expect(commands.some((command) => command.topic === "did.interrupt")).toBeTruthy();

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
      bodyLength: number;
      bodyText: string;
      completed?: boolean;
    }>;
  };
  const sttRequests = requests.filter((entry) => entry.kind === "stt");
  const llmRequests = requests.filter((entry) => entry.kind === "llm");
  const avatarRequests = requests.filter((entry) => entry.kind === "avatar");

  // Provider-call evidence is recorded when each Deepgram WebSocket opens. The interrupted
  // second turn may or may not reach CloseStream before cancellation, so completion is not
  // a stable boundary; opening the authorized stream is.
  expect(sttRequests).toHaveLength(2);
  expect(sttRequests.every((entry) => entry.method === "WEBSOCKET")).toBeTruthy();
  expect(sttRequests.every((entry) => entry.path === "/v1/listen")).toBeTruthy();
  expect(sttRequests.every((entry) => entry.query.includes("model=nova-3"))).toBeTruthy();
  expect(sttRequests.every((entry) => entry.query.includes("encoding=linear16"))).toBeTruthy();
  expect(sttRequests.every((entry) => entry.query.includes("sample_rate=16000"))).toBeTruthy();
  expect(sttRequests.every((entry) => entry.query.includes("channels=1"))).toBeTruthy();
  expect(sttRequests.every((entry) => entry.query.includes("interim_results=true"))).toBeTruthy();
  expect(sttRequests.every((entry) => entry.query.includes("smart_format=true"))).toBeTruthy();
  expect(sttRequests.every((entry) => entry.query.includes("language=ru"))).toBeTruthy();
  expect(sttRequests.every(
    (entry) => entry.authorization === "Token expressive-stt-e2e-secret",
  )).toBeTruthy();
  expect(sttRequests.every((entry) => entry.contentType === null)).toBeTruthy();
  expect(sttRequests[0]?.completed).toBe(true);
  expect(sttRequests[0]?.bodyLength ?? 0).toBeGreaterThan(0);

  // The browser contract ends at canonical interrupt: cancellation may beat LLM dispatch,
  // or it may cancel an already-open LLM stream. Both are correct as long as no second did.speak
  // escapes. Open-stream tail cancellation is proven deterministically in Rust state tests.
  expect(llmRequests.length).toBeGreaterThanOrEqual(1);
  expect(llmRequests.length).toBeLessThanOrEqual(2);
  expect(llmRequests.every(
    (entry) => entry.authorization === "Bearer expressive-llm-e2e-secret",
  )).toBeTruthy();
  expect(llmRequests[0]?.bodyText).toContain('"model":"deepseek-flash"');
  expect(llmRequests[0]?.bodyText).toContain('"reasoning_effort":"none"');
  expect(llmRequests[0]?.bodyText).toContain('"thinking":{"type":"disabled"}');
  expect(llmRequests[0]?.bodyText).toContain('"max_tokens":96');
  if (llmRequests[1]) {
    expect(llmRequests[1].bodyText).toContain("Что думает владелец?");
  }

  expect(avatarRequests.some((entry) =>
    entry.method === "GET" && entry.path === "/agents/voice-e2e-expressive-agent"
  )).toBeTruthy();
  expect(avatarRequests.some((entry) =>
    entry.method === "POST"
      && entry.path === "/v2/agents/voice-e2e-expressive-agent/sessions"
  )).toBeTruthy();
  expect(avatarRequests.some((entry) => entry.path.includes("/streams"))).toBeFalsy();

  // The same stale request can no longer prepare a speak after disconnect.
  const denied = await postJson(request, csrf, "/api/avatar/resume-answer", {
    request_sequence: 1,
    sentence_index: 0,
  });
  expect(denied.status()).toBe(409);
  const invalid = await postJson(request, csrf, "/api/avatar/resume-answer", {
    request_sequence: 1,
    sentence_index: 999,
  });
  expect(invalid.status()).toBe(409);
  const providerAfterDeny = await request.get(`${providerUrl}/__state`);
  expect(providerAfterDeny.ok()).toBeTruthy();
  const afterRequests = (await providerAfterDeny.json() as { requests: unknown[] }).requests;
  expect(afterRequests).toHaveLength(requests.length);
});