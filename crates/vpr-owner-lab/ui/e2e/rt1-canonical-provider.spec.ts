import { expect, test } from "@playwright/test";

const base = "http://127.0.0.1:18787";
const provider = "http://127.0.0.1:18788";
const csrfHeaders = (csrf: string) => ({
  "content-type": "application/json",
  "x-vpr-csrf": csrf,
  origin: base,
});

test("RT1 visitor reuses canonical D-ID provider for create, answer and confirmed close", async ({ page, request }) => {
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

  const start = await call("/api/rt1/visitor/start", { visitor_token, consent: true });
  expect(start.status, JSON.stringify(start.body)).toBe(200);
  expect(start.body.signal.transport.kind).toBe("web_rtc");
  expect(start.body.signal.transport.offer.kind).toBe("offer");
  expect(start.body.signal.transport.offer.sdp).toContain("backend-e2e-");
  const session_id = String(start.body.session_id);

  // A regular D-ID WebRTC avatar has no independent private Echo STOP.
  // RT1 must never send visitor microphone audio to that unfenced adapter.
  const unsupportedVoice = await call("/api/rt1/visitor/voice", {
    session_id,
    pcm: [0, 0, 0, 0],
  });
  expect(unsupportedVoice.status).toBe(409);


  const answer = await call("/api/rt1/visitor/answer", {
    session_id,
    sdp: "v=0 canonical-browser-answer",
  });
  expect(answer.status).toBe(200);

  const ice = await call("/api/rt1/visitor/ice", {
    session_id,
    candidate: "candidate:1 1 udp 1 192.0.2.1 9000 typ host",
    sdp_mid: "0",
    sdp_mline_index: 0,
  });
  expect(ice.status, JSON.stringify(ice.body)).toBe(200);

  const requestsBefore = (await (await request.get(`${provider}/__state`)).json()).requests;
  const creates = requestsBefore.filter(
    (item: { method: string; path: string }) =>
      item.method === "POST" && item.path === "/agents/backend-e2e-agent/streams",
  );
  expect(creates).toHaveLength(1);
  const sdps = requestsBefore.filter(
    (item: { method: string; path: string }) =>
      item.method === "POST" && /\/streams\/stream-\d+\/sdp$/.test(item.path),
  );
  expect(sdps).toHaveLength(1);
  const candidates = requestsBefore.filter(
    (item: { method: string; path: string }) =>
      item.method === "POST" && /\/streams\/stream-\d+\/ice$/.test(item.path),
  );
  expect(candidates).toHaveLength(1);


  const pause = await call("/api/rt1/owner/pause", { owner_token });
  expect(pause.status).toBe(200);
  expect(pause.body.remote_stop_confirmed).toBe(false);
  const stopped = await call("/api/rt1/visitor/close", { session_id });
  expect(stopped.status, JSON.stringify(stopped.body)).toBe(200);
  expect(stopped.body.remote_stop_confirmed).toBe(true);

  const state = (await (await request.get(`${provider}/__state`)).json()).requests;
  const deletes = state.filter(
    (item: { method: string; path: string }) =>
      item.method === "DELETE" && /\/agents\/backend-e2e-agent\/streams\/stream-\d+$/.test(item.path),
  );
  expect(deletes).toHaveLength(1);
  expect(deletes[0].path).toBe(sdps[0].path.replace(/\/sdp$/, ""));
  const stale = await call("/api/rt1/visitor/start", { visitor_token, consent: true });
  expect(stale.status).not.toBe(200);
});
