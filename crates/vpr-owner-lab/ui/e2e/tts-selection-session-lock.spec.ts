import { expect, test } from "@playwright/test";

test("TTS selection honors the canonical session lock and confirmed provider", async ({ page }) => {
  let sessionActive = true;
  const submitted: string[] = [];
  let releaseFirst = () => {};
  const firstResponse = new Promise<void>(resolve => { releaseFirst = resolve; });

  await page.route("**/app.js", async route => {
    await route.fulfill({ status: 200, contentType: "text/javascript", body: "export {};" });
  });
  await page.route("**/api/bootstrap", async route => {
    await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ csrf_token: "test-csrf" }) });
  });
  await page.route("**/api/tts/providers", async route => {
    await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ active: "piper", alternate: "openai-speech" }) });
  });
  await page.route("**/api/tts/select", async route => {
    const request = route.request();
    expect(request.headers()["x-vpr-csrf"]).toBe("test-csrf");
    submitted.push(request.postDataJSON().provider);
    if (sessionActive) await firstResponse;
    await route.fulfill({
      status: sessionActive ? 409 : 200,
      contentType: "application/json",
      body: JSON.stringify(sessionActive
        ? { error: "TTS_LOCKED_DURING_SESSION" }
        : { active: "openai-speech", alternate: "piper" }),
    });
  });

  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.addScriptTag({ url: "/tts-selector.js", type: "module" });
  const status = page.locator("#tts-switch-status");
  await expect(page.locator("#tts-switch-panel")).toBeVisible();
  await expect(status).toContainText("после завершения текущего сеанса");
  await page.locator("#tts-switch-provider").selectOption("openai-speech");
  await page.locator("#tts-switch-apply").click();
  await expect(page.locator("#tts-switch-provider")).toBeDisabled();
  await expect(page.locator("#tts-switch-apply")).toBeDisabled();
  releaseFirst();
  await expect(status).toContainText("Завершите текущий сеанс");
  expect(submitted).toEqual(["openai-speech"]);

  sessionActive = false;
  await page.locator("#tts-switch-apply").click();
  await expect(status).toHaveText("Теперь активен: openai-speech");
  expect(submitted).toEqual(["openai-speech", "openai-speech"]);
});
