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
  await page.locator("#persona-version").fill("2");
  await page.locator("#publish").click();

  await expect(page.locator("#status")).toContainText("RT1_VIDEO_RENDERER_UNAVAILABLE", {
    timeout: 20_000,
  });
  await expect(page.locator("#invite-token")).toHaveValue("");
});
