import {
  expect,
  test,
  type APIRequestContext,
} from "@playwright/test";

import { installProviderAutoConnect } from "./provider-bootstrap.js";

const ownerLabUrl = "http://127.0.0.1:18791";
const providerUrl = "http://127.0.0.1:18790";
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

test("Expressive LiveKit voice path reaches canonical playback, A/V sync and recovery", async ({
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
  await page.route("**/__expressive_journey_report", async (route) => {
    const incoming = route.request();
    if (incoming.method() !== "POST") {
      await route.fulfill({ status: 405 });
      return;
    }
    try {
      const payload = JSON.parse(incoming.postData() ?? "") as
        | ExpressiveJourneyReport
        | { kind: "phase"; phase: string };
      if ("kind" in payload && payload.kind === "phase") {
        lastJourneyPhase = payload.phase;
      } else {
        report = payload as ExpressiveJourneyReport;
      }
    } catch {
      report = { status: "failed", error: "INVALID_EXPRESSIVE_JOURNEY_REPORT" };
    }
    await route.fulfill({ status: 204 });
  });

  await page.addInitScript({ path: "e2e/fake-livekit-client.js" });
  await installProviderAutoConnect(page);
  await page.addInitScript({ path: "e2e/expressive-journey-driver.js" });
  await page.route("**/__expressive_provider_state", async (route) => {
    const incoming = route.request();
    if (incoming.method() !== "GET") {
      await route.fulfill({ status: 405 });
      return;
    }
    const providerState = await request.get(`${providerUrl}/__state`);
    await route.fulfill({
      status: providerState.status(),
      contentType: "application/json",
      body: await providerState.text(),
    });
  });
  await page.route("**/api/evidence/media", async (route) => {
    const incoming = route.request();
    if (incoming.method() === "POST") {
      const body = incoming.postDataJSON() as { kind?: string } | null;
      if (body?.kind === "audio_started") {
        await new Promise<void>((resolve) => setTimeout(resolve, 300));
      }
    }
    await route.continue();
  });
  await page.goto("/");

  await expect.poll(() => {
    if (!report) return `pending:${lastJourneyPhase}`;
    return report.status === "failed"
      ? `failed:${report.error ?? "unknown"}@phase:${lastJourneyPhase}`
      : report.status;
  }, {
    timeout: EXPRESSIVE_JOURNEY_COMPLETION_TIMEOUT_MS,
    intervals: [100, 250, 500],
    message: "Expressive journey must publish a terminal same-origin report",
  }).toBe("ok");

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
  expect(evidence?.canonical_playback_proven).toBeTruthy();
  expect(evidence?.av_sync_proven).toBeTruthy();
  expect(evidence?.av_sync_samples).toHaveLength(3);
  expect(evidence?.av_sync_samples.map((sample) => sample.sample_sequence)).toEqual([1, 2, 3]);
  expect(evidence?.av_sync_samples.every((sample) =>
    sample.reference === "web_rtc_estimated_playout_timestamp"
      && sample.absolute_offset_millis === 60
  )).toBeTruthy();
  expect(evidence?.voice_attempts.some((attempt) =>
    attempt.status === "completed" && attempt.canonical_playback_confirmed
  )).toBeTruthy();

  const metrics = report?.metrics;
  expect(metrics).toBeDefined();
  expect(metrics?.stt).toMatch(/\d+ мс/);
  expect(metrics?.llm).toMatch(/\d+ мс/);
  expect(metrics?.llmFirst).toMatch(/\d+ мс/);
  expect(metrics?.serverTotal).toMatch(/\d+ мс/);
  expect(metrics?.firstAudio).toMatch(/\d+ мс/);
  expect(metrics?.videoReady).toMatch(/\d+ мс/);
  expect(metrics?.avSync).toBe("60 мс · 3 изм.");
  expect(metrics?.playback).toBe("подтверждён");
  expect(metrics?.cost).toBe("провайдер не сообщил стоимость");

  const commands = report?.commands ?? [];
  const speak = commands.filter((command) => command.topic === "did.speak");
  expect(speak).toHaveLength(1);
  expect(JSON.parse(speak[0]?.text ?? "{}").script.input).toBe(
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
