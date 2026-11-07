import { expect, test } from "@playwright/test";

const base = "http://127.0.0.1:18787";
const provider = "http://127.0.0.1:18788";
const csrfHeaders = (csrf: string) => ({
  "content-type": "application/json",
  "x-vpr-csrf": csrf,
  origin: base,
});

test("RT1 visitor reuses canonical D-ID create/answer and quarantines unconfirmed remote close", async ({ page, request }) => {
  // Module scripts must execute before we interpret Owner Lab bootstrap status.
  // Keep the real browser/assets/backend path: no mocked bootstrap or retries.
  const browserErrors: string[] = [];
  const failedRequests: string[] = [];
  const pendingRequests = new Set<string>();
  page.on("pageerror", (error) => browserErrors.push(error.message));
  page.on("request", (request) => pendingRequests.add(request.url()));
  page.on("requestfinished", (request) => pendingRequests.delete(request.url()));
  page.on("requestfailed", (request) => {
    pendingRequests.delete(request.url());
    failedRequests.push(`${request.url()}: ${request.failure()?.errorText ?? "unknown"}`);
  });
  page.on("response", (response) => {
    if (response.status() >= 400 && new URL(response.url()).origin === base) {
      failedRequests.push(`${response.status()} ${response.url()}`);
    }
  });
  try {
    await page.goto("/", { waitUntil: "commit", timeout: 30_000 });
    await expect(page.locator("#status")).not.toHaveText("Загрузка…", { timeout: 25_000 });
  } catch (error) {
    throw new Error(`Canonical Owner Lab did not initialize; browserErrors=${JSON.stringify(browserErrors)}; pendingRequests=${JSON.stringify([...pendingRequests])}; failedRequests=${JSON.stringify(failedRequests)}; assertion=${String(error)}`);
  }
  expect(browserErrors, "Owner Lab must initialize without uncaught browser exceptions").toEqual([]);
  await page.getByLabel("Идентификатор Persona").fill("canonical-rt1-e2e");
  await page.getByRole("button", { name: "Создать Persona" }).click();
  const answers = ["Разговариваю спокойно", "Предпочитаю точность", "Объясняю кратко"];
  for (const [index, answer] of answers.entries()) {
    await page.getByPlaceholder("Ответьте своими словами").fill(answer);
    await page.getByRole("button", { name: "Сохранить ответ" }).click();
    await expect(page.locator("#persona-progress")).toContainText(`${index + 1}/${answers.length}`);
  }
  await page.getByRole("button", { name: "Перейти к проверке" }).click();
  const accept = page.getByRole("button", { name: "Подтвердить без изменений" });
  for (let i = 0; i < answers.length; i += 1) {
    await accept.first().click();
  }
  await page.getByRole("button", { name: "Подтвердить Persona" }).click();
  await expect(page.locator("#persona-progress")).toContainText("версия 2");

  const boot = await request.get(`${base}/api/bootstrap`);
  expect(boot.ok()).toBeTruthy();
  const csrf = String((await boot.json()).csrf_token);
  const call = async (path: string, data: object) => {
    const result = await request.post(`${base}${path}`, { headers: csrfHeaders(csrf), data });
    return { status: result.status(), body: await result.json() };
  };
  const provision = await call("/api/rt1/owner/provision", {});
  expect(provision.status).toBe(200);
  const owner_token = String(provision.body.owner_token);
  const publish = await call("/api/rt1/owner/publish", {
    owner_token,
    expected_persona_version: 2,
  });
  expect(publish.status).toBe(200);
  const visitor_token = String(publish.body.visitor_token);

  // D-ID cannot confirm independent remote video STOP. Never CREATE a remote
  // session or issue visitor voice/text egress while this safety gate is unmet.
  const start = await call("/api/rt1/visitor/start", { visitor_token, consent: true });
  expect(start.status, JSON.stringify(start.body)).toBe(503);
  expect(start.body.code).toBe("RT1_REMOTE_VIDEO_STOP_UNCONFIRMED");
  expect(start.body).not.toHaveProperty("session_id");

  const forgedText = await request.post(`${base}/api/rt1/visitor/text`, {
    headers: { "content-type": "application/json", origin: base },
    data: { session_id: "forged-session", text: "CSRF bypass attempt" },
  });
  expect(forgedText.status()).toBe(403);
  expect((await forgedText.json()).code).toBe("CSRF_DENIED");

  const deniedText = await call("/api/rt1/visitor/text", {
    session_id: "forged-session", text: "No unfenced LLM egress",
  });
  expect(deniedText.status).not.toBe(200);
  expect(deniedText.body).not.toHaveProperty("reply");
  const deniedVoice = await call("/api/rt1/visitor/voice", {
    session_id: "forged-session", pcm: [0, 0, 0, 0],
  });
  expect(deniedVoice.status).not.toBe(200);

  const providerRequests = async () =>
    (await (await request.get(`${provider}/__state`)).json()).requests as
      { method: string; path: string }[];
  const creates = (await providerRequests()).filter(
    (item) => item.method === "POST" && item.path === "/agents/backend-e2e-agent/streams",
  );
  expect(creates).toHaveLength(0);

  const pause = await call("/api/rt1/owner/pause", { owner_token });
  expect(pause.status).toBe(200);
  const stale = await call("/api/rt1/visitor/start", { visitor_token, consent: true });
  expect(stale.status).not.toBe(200);
  expect((await providerRequests()).filter(
    (item) => item.method === "POST" && item.path === "/agents/backend-e2e-agent/streams",
  )).toHaveLength(0);
});
