import { expect, test } from "@playwright/test";

test("RT1 refuses a public visitor invite when only diagnostic video transport is configured", async ({ page }) => {
  const failedAssets: string[] = [];
  page.on("requestfailed", (request) => {
    failedAssets.push(`${request.url()}: ${request.failure()?.errorText ?? "unknown"}`);
  });

  await page.goto("/", { waitUntil: "commit", timeout: 25_000 });
  try {
    await expect(page.locator("#workflow-state"))
      .toContainText("Ожидает проверки владельцем", { timeout: 20_000 });
    await expect(page.locator("#status")).not.toContainText("Интерфейс не загрузился");
  } catch (firstFailure) {
    const bootstrap = await page.request.get("/api/bootstrap", { timeout: 10_000 });
    expect(bootstrap.ok(), `Owner Lab backend unavailable: ${bootstrap.status()}`).toBeTruthy();
    await page.reload({ waitUntil: "commit", timeout: 25_000 });
    try {
      await expect(page.locator("#workflow-state"))
        .toContainText("Ожидает проверки владельцем", { timeout: 25_000 });
      await expect(page.locator("#status")).not.toContainText("Интерфейс не загрузился");
    } catch (secondFailure) {
      throw new Error(
        `Owner Lab bootstrap failed after a real navigation retry; requests=${JSON.stringify(failedAssets)}; ` +
        `first=${String(firstFailure)}; second=${String(secondFailure)}`,
      );
    }
  }

  await page.getByLabel("Идентификатор Persona").fill("rt1-diagnostic-video-denial");
  await page.getByRole("button", { name: "Создать Persona" }).click();
  const answers = ["Слушаю внимательно", "Говорю спокойно", "Отвечаю кратко"];
  for (const [index, answer] of answers.entries()) {
    await page.getByPlaceholder("Ответьте своими словами").fill(answer);
    await page.getByRole("button", { name: "Сохранить ответ" }).click();
    await expect(page.locator("#persona-progress")).toContainText(`${index + 1}/${answers.length}`);
  }
  await page.getByRole("button", { name: "Перейти к проверке" }).click();
  const accept = page.getByRole("button", { name: "Подтвердить без изменений" });
  await expect(accept).toHaveCount(answers.length);
  for (let index = 0; index < answers.length; index += 1) await accept.first().click();
  await page.getByRole("button", { name: "Подтвердить Persona" }).click();
  await expect(page.locator("#persona-progress")).toContainText("версия 2");

  // The browser reaches the real owner publication endpoint. The test worker
  // has only diagnostic aiortc video; it must not receive a visitor token.
  await page.goto("/rt1", { waitUntil: "commit", timeout: 25_000 });
  await expect(page.locator("#status")).toHaveAttribute("data-rt1-ready", "true", { timeout: 20_000 });
  await expect(page.getByRole("heading", { name: "Persona · RT1" })).toBeVisible();
  await page.locator("#provision").click();
  await expect(page.locator("#status")).toContainText("Кабинет создан");
  const ownerToken = await page.locator("#owner-token").inputValue();
  expect(ownerToken.length).toBeGreaterThan(30);
  await expect(page.locator("#owner-state")).toContainText("rt1-diagnostic-video-denial");
  // Reload must never persist the owner secret, but reentering a legitimate
  // owner token restores the correct canonical version and publication state.
  await page.reload({ waitUntil: "domcontentloaded" });
  await expect(page.locator("#owner-token")).toHaveValue("");
  await expect(page.locator("#publish")).toBeDisabled();
  await page.locator("#owner-token").fill(ownerToken);
  await page.locator("#owner-refresh").click();
  await expect(page.locator("#publish")).toBeEnabled();
  await expect(page.locator("#persona-version")).toHaveValue("2");
  await expect(page.locator("#persona-version")).toHaveAttribute("readonly", "");
  // Browser authority race: a delayed successful publish reply must never
  // display the former owner's visitor bearer after the key was edited.
  // This intercepts only the single publication response, not real server
  // admission/STOP; the diagnostic-video denial remains checked below.
  let releasePublish!: () => void;
  const heldPublish = new Promise<void>((resolve) => { releasePublish = resolve; });
  const testInvite = "b".repeat(64);
  await page.route("**/api/rt1/owner/publish", async (route) => {
    await heldPublish;
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ visitor_token: testInvite }),
    });
  });
  const publishRequest = page.waitForRequest((request) =>
    request.url().endsWith("/api/rt1/owner/publish") && request.method() === "POST",
  );
  await page.locator("#publish").click();
  await publishRequest;
  await page.locator("#owner-token").fill("another-owner-token");
  // Returning to the same token must not authorize the stale response.
  await page.locator("#owner-token").fill(ownerToken);
  const publishResponse = page.waitForResponse((response) =>
    response.url().endsWith("/api/rt1/owner/publish") && response.request().method() === "POST",
  );
  releasePublish();
  await publishResponse;
  // Switching credentials invalidates the canonical owner snapshot. The
  // publish button must stay disabled until an explicit owner refresh.
  await expect(page.locator("#publish")).toBeDisabled();
  await expect(page.locator("#invite-token")).toHaveValue("");
  await expect(page.locator("#invite-link")).toHaveValue("");
  await expect(page.locator("#copy-invite")).toBeDisabled();
  await page.unroute("**/api/rt1/owner/publish");
  await page.locator("#owner-refresh").click();
  await expect(page.locator("#publish")).toBeEnabled();
  await page.locator("#publish").click();

  await expect(page.locator("#status")).toContainText("RT1_VIDEO_RENDERER_UNAVAILABLE", {
    timeout: 20_000,
  });
  await expect(page.locator("#invite-token")).toHaveValue("");
});


test("RT1 owner invite uses the authenticated public HTTPS origin and visitor bootstrap hides owner metadata", async ({ page }) => {
  await page.goto("/rt1", { waitUntil: "domcontentloaded" });
  const minimalBootstrap = await page.request.get("/api/rt1/visitor/bootstrap");
  expect(minimalBootstrap.status()).toBe(200);
  const bootstrap = await minimalBootstrap.json();
  expect(Object.keys(bootstrap).sort()).toEqual(["csrf_token"]);
  expect(typeof bootstrap.csrf_token).toBe("string");

  const bearer = "q".repeat(64);
  let published = false;
  await page.route("**/api/rt1/owner/status", (route) =>
    route.fulfill({
      status: 200, contentType: "application/json",
      body: JSON.stringify({
        owner_id: "owner-1",
        persona_id: "reviewed-persona",
        persona_version: 2,
        publication_status: published ? "published" : "private",
        public_visitor_origin: "https://visitor.example.org",
      }),
    }));
  await page.route("**/api/rt1/owner/publish", (route) => {
    published = true;
    return route.fulfill({
      status: 200, contentType: "application/json",
      body: JSON.stringify({ visitor_token: bearer }),
    });
  });

  await page.locator("#owner-token").fill("test-owner-credential");
  await page.locator("#owner-refresh").click();
  await expect(page.locator("#publish")).toBeEnabled();
  await page.locator("#publish").click();
  await expect(page.locator("#invite-link")).toHaveValue(
    "https://visitor.example.org/rt1/visit#invite=" + bearer,
  );
  // The URL's path contains no bearer; the browser consumes the fragment
  // before any fetch, so a public gateway never logs it in request paths.
  const link = await page.locator("#invite-link").inputValue();
  expect(new URL(link).pathname).toBe("/rt1/visit");
  expect(new URL(link).search).toBe("");
  expect(new URL(link).hash).toBe("#invite=" + bearer);
});
