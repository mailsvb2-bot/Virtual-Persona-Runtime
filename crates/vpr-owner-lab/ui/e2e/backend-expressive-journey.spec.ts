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

// All Expressive journeys share one real Owner Lab backend. Closing alone
// does NOT release its strict evidence-export gate: the next real Connect
// must not bypass or silently discard a previous session's proof. This
// teardown closes and exports through the same canonical HTTP paths as UI.
test.afterEach(async ({ request }) => {
  const bootstrap = await request.get(`${ownerLabUrl}/api/bootstrap`);
  if (!bootstrap.ok()) return;
  const csrf = String((await bootstrap.json()).csrf_token);
  const status = await request.get(`${ownerLabUrl}/api/status`);
  if (!status.ok()) return;
  const state = (await status.json() as { session_state: string }).session_state;
  if (state === "none") return;
  if (state !== "closed") {
    const close = await postJson(request, csrf, "/api/session/close", {});
    expect(close.ok(), "E2E teardown must close the provider before exporting").toBeTruthy();
  }
  const exported = await postJson(request, csrf, "/api/evidence/session/export", {});
  expect(exported.ok(), "E2E teardown must satisfy the next session's evidence export gate").toBeTruthy();
  const evidence = await exported.json() as { session_sequence?: number };
  expect(evidence.session_sequence).toBeGreaterThan(0);
});

// The fixture launches a real server-owned Echo worker with a hermetic
// LiveKit transport. The browser must receive only a viewer token.
test("Expressive Echo keeps its sender credentials private", async ({
  page,
  request,
}) => {
  const bootstrap = await request.get(`${ownerLabUrl}/api/bootstrap`);
  expect(bootstrap.ok()).toBeTruthy();
  const csrf = String((await bootstrap.json()).csrf_token);
  await setupReviewedPersona(request, csrf);

  const started = await postJson(request, csrf, "/api/avatar/start", { consent: true });
  expect(started.status()).toBe(200);
  const payload = await started.text();
  expect(payload).not.toContain("fixture-private-echo-token");
  expect(payload).not.toContain("echo_token");
  expect(payload).not.toContain("did.speak");
  const session = JSON.parse(payload) as {
    transport: { kind: string; token?: string };
    client_control?: { text_input?: boolean } | null;
  };
  expect(session.transport.kind).toBe("live_kit");
  expect(session.transport.token).toMatch(/^fixture-livekit-token-/);
  expect(session.client_control?.text_input ?? false).toBe(false);
  const status = await request.get(`${ownerLabUrl}/api/status`);
  expect(status.ok()).toBeTruthy();
  expect(await status.json()).toMatchObject({
    session_state: "active",
    avatar_open: true,
  });
  // Exercise a genuine browser navigation without post-navigation renderer RPC.
  await page.goto("/");
});

// Echo speech is published from the actual backend Python process; observe
// the local provider fixture, not obsolete browser data-channel commands.
type EchoEvent = { kind: string; event?: string };
const echoEvents = async (request: APIRequestContext): Promise<string[]> => {
  const response = await request.get(`${providerUrl}/__state`);
  expect(response.ok()).toBeTruthy();
  const state = await response.json() as { requests: EchoEvent[] };
  return state.requests.filter((entry) => entry.kind === "echo").map((entry) => String(entry.event));
};
const startEcho = async (request: APIRequestContext) => {
  const bootstrap = await request.get(`${ownerLabUrl}/api/bootstrap`);
  expect(bootstrap.ok()).toBeTruthy();
  const csrf = String((await bootstrap.json()).csrf_token);
  await setupReviewedPersona(request, csrf);
  const started = await postJson(request, csrf, "/api/avatar/start", { consent: true });
  expect(started.status()).toBe(200);
  const session = await started.json() as { evidence_session_sequence: number; transport: { kind: string; token: string }; client_control?: { text_input?: boolean } };
  expect(session.transport.kind).toBe("live_kit");
  expect(session.client_control?.text_input ?? false).toBe(false);
  return { csrf, sequence: session.evidence_session_sequence };
};
const submitOwnerTurn = async (request: APIRequestContext, csrf: string, sequence: number, text: string) =>
  request.post(`${ownerLabUrl}/api/text/turn`, {
    headers: { ...csrfHeaders(csrf), "X-VPR-Evidence-Request": String(sequence) },
    data: { text },
  });

test("Expressive server Echo publishes synthesized voice without browser did.speak", async ({ request }) => {
  test.setTimeout(90000);
  const { csrf } = await startEcho(request);
  const before = (await echoEvents(request)).length;
  const reply = await submitOwnerTurn(request, csrf, 1, "Привет из браузера");
  expect(reply.status()).toBe(200);
  await expect.poll(async () => (await echoEvents(request)).slice(before), {
    timeout: 30000,
  }).toEqual(expect.arrayContaining(["audio_stream_opened", "audio_bytes_written", "audio_stream_closed"]));
  const evidence = await request.get(`${ownerLabUrl}/api/evidence/session`);
  expect(evidence.ok()).toBeTruthy();
  expect(await evidence.json()).toMatchObject({ canonical_playback_proven: false });
});

test("Expressive server STOP is sent from backend and prevents stale replay", async ({ request }) => {
  test.setTimeout(90000);
  const { csrf, sequence } = await startEcho(request);
  const before = (await echoEvents(request)).length;
  const reply = await submitOwnerTurn(request, csrf, 1, "Привет из браузера");
  expect(reply.status()).toBe(200);
  const stopped = await postJson(request, csrf, "/api/avatar/interrupt", { expected_session_sequence: sequence });
  expect(stopped.ok()).toBeTruthy();
  await expect.poll(async () => (await echoEvents(request)).slice(before), { timeout: 15000 })
    .toEqual(expect.arrayContaining(["provider_stop_sent"]));
  const revoke = await postJson(request, csrf, "/api/session/revoke", {});
  expect(revoke.ok()).toBeTruthy();
  const replay = await postJson(request, csrf, "/api/avatar/resume-answer", { request_sequence: 1, sentence_index: 0 });
  expect(replay.status()).toBe(409);
});

test("Expressive server Echo closes privately on session revoke", async ({ request }) => {
  test.setTimeout(90000);
  const { csrf } = await startEcho(request);
  const before = (await echoEvents(request)).length;
  const turn = await submitOwnerTurn(request, csrf, 1, "Привет из браузера");
  expect(turn.ok()).toBeTruthy();
  const revoke = await postJson(request, csrf, "/api/session/revoke", {});
  expect(revoke.ok()).toBeTruthy();
  await expect.poll(async () => (await echoEvents(request)).slice(before), { timeout: 15000 })
    .toEqual(expect.arrayContaining(["private_echo_disconnected"]));
  const blocked = await submitOwnerTurn(request, csrf, 2, "Запрещённая речь после отзыва");
  expect(blocked.ok()).toBeFalsy();
});

test("second caller cannot restart speech after canonical revoke", async ({ request }) => {
  const { csrf } = await startEcho(request);
  const revoked = await postJson(request, csrf, "/api/session/revoke", {});
  expect(revoked.ok()).toBeTruthy();
  const before = await echoEvents(request);
  const rejected = await postJson(request, csrf, "/api/avatar/resume-answer", { request_sequence: 1, sentence_index: 0 });
  expect(rejected.status()).toBe(409);
  const rejectedTurn = await submitOwnerTurn(request, csrf, 1, "Несанкционированный ответ");
  expect(rejectedTurn.ok()).toBeFalsy();
  expect(await echoEvents(request)).toEqual(before);
});
