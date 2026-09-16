import { expect, test, type Page } from "@playwright/test";

const ownerLabUrl = "http://127.0.0.1:18787";
const providerUrl = "http://127.0.0.1:18788";
const answers = [
  "Люблю быстрые итерации",
  "Мне важна точность",
  "Предпочитаю спокойный тон",
];

const installTransportFakes = async (page: Page): Promise<void> => {
  await page.addInitScript(() => {
    class FakeAudioContext {
      sampleRate = 48_000;
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
        return { type: "answer", sdp: "v=0 backend-browser-answer" };
      }
      async setLocalDescription(): Promise<void> {}
      close(): void { this.connectionState = "closed"; }
    }
    Object.defineProperty(window, "AudioContext", { value: FakeAudioContext });
    Object.defineProperty(window, "RTCPeerConnection", { value: FakePeerConnection });
  });
};

const csrfHeaders = (csrf: string) => ({
  "content-type": "application/json",
  origin: ownerLabUrl,
  "x-vpr-csrf": csrf,
});

test("built UI drives the real Owner Lab backend and provider adapter", async ({ page, request }) => {
  await installTransportFakes(page);
  await page.goto("/");

  const bootstrap = await request.get(`${ownerLabUrl}/api/bootstrap`);
  expect(bootstrap.ok()).toBeTruthy();
  const csrf = String((await bootstrap.json()).csrf_token);

  await expect(page.getByRole("button", { name: "Подключить аватар" })).toBeDisabled();
  await page.getByLabel("Идентификатор Persona").fill("owner-backend-e2e");
  await page.getByRole("button", { name: "Создать Persona" }).click();
  for (const [index, answer] of answers.entries()) {
    await page.getByPlaceholder("Ответьте своими словами").fill(answer);
    await page.getByRole("button", { name: "Сохранить ответ" }).click();
    await expect(page.locator("#persona-progress")).toContainText(`${index + 1}/${answers.length}`);
  }

  const approveButtons = page.getByRole("button", { name: "Подтвердить без изменений" });
  await page.getByRole("button", { name: "Перейти к проверке" }).click();
  await expect(approveButtons).toHaveCount(answers.length);
  for (let index = 0; index < answers.length; index += 1) {
    await approveButtons.first().click();
    await expect(approveButtons).toHaveCount(answers.length - index - 1);
  }
  await page.getByRole("button", { name: "Подтвердить Persona" }).click();
  await expect(page.locator("#persona-progress")).toContainText("версия 2");

  await page.getByRole("checkbox").check();
  await page.getByRole("button", { name: "Подключить аватар" }).click();
  await expect(page.locator("#status")).toContainText("WebRTC согласован");
  await page.getByLabel("Что должен сказать аватар").fill("Проверка реального backend пути");
  await page.getByRole("button", { name: "Сказать", exact: true }).click();
  const prematureExport = await request.post(`${ownerLabUrl}/api/evidence/session/export`, {
    headers: csrfHeaders(csrf),
    data: {},
  });
  expect(prematureExport.status()).toBe(409);
  expect(await prematureExport.json()).toMatchObject({ code: "EVIDENCE_SESSION_NOT_TERMINAL" });
  await page.getByRole("button", { name: "Закрыть" }).click();
  await expect(page.locator("#status")).toContainText("Сессия закрыта");
  const blockedNextSession = await request.post(`${ownerLabUrl}/api/avatar/start`, {
    headers: csrfHeaders(csrf),
    data: { consent: true, audience: "visitor" },
  });
  expect(blockedNextSession.status()).toBe(409);
  expect(await blockedNextSession.json()).toMatchObject({ code: "EVIDENCE_EXPORT_REQUIRED" });
  const ownerEvidenceDownloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: "Скачать evidence snapshot" }).click();
  const ownerEvidenceDownload = await ownerEvidenceDownloadPromise;
  expect(ownerEvidenceDownload.suggestedFilename()).toBe("session-1-owner.json");
  const reviewedProfile = await request.post(`${ownerLabUrl}/api/persona/reviewed`, {
    headers: csrfHeaders(csrf),
    data: {},
  });
  expect(reviewedProfile.ok()).toBeTruthy();
  const reviewedJson = await reviewedProfile.json() as {
    claims: Array<{ claim_id: string; statement: string }>;
  };
  expect(reviewedJson.claims.map((claim) => claim.claim_id)).toEqual([
    "identity-self-description",
    "preference-communication-style",
    "opinion-core-principle",
  ]);
  const claimToCorrect = reviewedJson.claims.find((claim) => claim.statement === answers[2]);
  expect(claimToCorrect).toBeTruthy();
  const ownerClaim = page.getByLabel(`Текущее утверждение ${claimToCorrect?.claim_id ?? ""}`);
  const ownerClaimCard = ownerClaim.locator("xpath=ancestor::article");
  const correctedStatement = "Точность важнее уверенного выдумывания";
  await ownerClaim.fill(correctedStatement);
  await ownerClaimCard.getByRole("button", { name: "Сохранить новую редакцию" }).click();
  await expect(page.locator("#persona-progress")).toContainText("версия 3");
  const correctedProfile = await request.post(`${ownerLabUrl}/api/persona/reviewed`, {
    headers: csrfHeaders(csrf),
    data: {},
  });
  expect(correctedProfile.ok()).toBeTruthy();
  const correctedJson = await correctedProfile.json() as {
    persona_version: number;
    claims: Array<{ claim_id: string; statement: string }>;
  };
  expect(correctedJson.persona_version).toBe(3);
  expect(correctedJson.claims.find((claim) => claim.claim_id === claimToCorrect?.claim_id)?.statement)
    .toBe(correctedStatement);

  await page.getByLabel("Режим тестовой сессии").selectOption("visitor");
  await expect(page.locator("#persona-panel")).toBeHidden();
  await page.getByRole("button", { name: "Подключить аватар" }).click();
  await expect(page.locator("#status")).toContainText("Visitor-сессия WebRTC согласована");

  const reviewed = await request.post(`${ownerLabUrl}/api/persona/reviewed`, {
    headers: csrfHeaders(csrf),
    data: {},
  });
  expect(reviewed.status()).toBe(403);
  expect(await reviewed.json()).toMatchObject({ code: "AUTH_SCOPE_DENIED" });

  const injectedSpeech = await request.post(`${ownerLabUrl}/api/avatar/speak`, {
    headers: csrfHeaders(csrf),
    data: { text: "Попытка прямой речи visitor" },
  });
  expect(injectedSpeech.status()).toBe(403);
  expect(await injectedSpeech.json()).toMatchObject({ code: "AUTH_SCOPE_DENIED" });
  await page.getByRole("button", { name: "Отозвать доступ" }).click();
  await expect(page.locator("#status")).toContainText("Доступ отозван");
  await page.getByRole("button", { name: "Закрыть" }).click();
  await expect(page.locator("#status")).toContainText("Сессия закрыта");
  const visitorEvidenceDownloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: "Скачать evidence snapshot" }).click();
  const visitorEvidenceDownload = await visitorEvidenceDownloadPromise;
  expect(visitorEvidenceDownload.suggestedFilename()).toBe("session-2-visitor.json");

  const backendStatus = await request.get(`${ownerLabUrl}/api/status`);
  const backendJson = await backendStatus.json();
  expect(backendJson).toMatchObject({
    session_state: "closed",
    session_audience: null,
    persona_version: 3,
  });

  const providerState = await request.get(`${providerUrl}/__state`);
  const { requests } = await providerState.json() as {
    requests: Array<{ method: string; path: string; authorization: string | null; body: string }>;
  };
  expect(requests.filter((entry) => entry.path.endsWith("/streams"))).toHaveLength(2);
  expect(requests.filter((entry) => entry.path.endsWith("/sdp"))).toHaveLength(2);
  expect(requests.filter((entry) => entry.method === "DELETE")).toHaveLength(2);
  const speech = requests.find((entry) => entry.method === "POST" && entry.path.endsWith("/stream-1"));
  expect(speech?.body).toContain("Проверка реального backend пути");
  expect(requests.every((entry) => entry.authorization === "Basic backend-e2e-secret")).toBeTruthy();
});
