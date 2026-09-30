import {
  expect,
  test,
  type APIRequestContext,
  type Page,
} from "@playwright/test";

import { installProviderAutoConnect } from "./provider-bootstrap.js";
import {
  assertBrowserJourneyEvidence,
  assertProviderRequests,
  waitForBrowserJourney,
} from "./voice-journey-contract.js";

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

const installVoiceJourney = async (page: Page): Promise<void> => {
  await page.addInitScript({ path: "e2e/fake-webrtc-media-runtime.js" });
  await installProviderAutoConnect(page);
  await page.addInitScript({ path: "e2e/voice-journey-driver.js" });
};

test("owner and visitor voice turns cross the real backend with different context scopes", async ({
  page,
  request,
}) => {
  const bootstrap = await request.get(`${ownerLabUrl}/api/bootstrap`);
  expect(bootstrap.ok()).toBeTruthy();
  const csrf = String((await bootstrap.json()).csrf_token);
  await setupReviewedPersona(request, csrf);

  await installVoiceJourney(page);
  await page.goto("/");

  // No Playwright/CDP page RPC is allowed after navigation in this media-provider journey.
  // The in-page driver exercises the real DOM controls; assertions consume only external
  // backend/provider evidence so Chromium media scheduling cannot deadlock the test controller.
  const journey = await waitForBrowserJourney(request, providerUrl);
  assertBrowserJourneyEvidence(journey);
  await assertProviderRequests(request, providerUrl, ownerAnswers);
});
