import { expect, test } from "@playwright/test";

test("reviewed owner publishes, separate visitor receives real RTP, owner revoke stops it", async ({ browser }) => {
  const ownerContext = await browser.newContext();
  const visitorContext = await browser.newContext();
  const owner = await ownerContext.newPage();
  const visitor = await visitorContext.newPage();

  try {
    await owner.goto("/", { waitUntil: "commit", timeout: 25_000 });
    await expect(owner.locator("#status")).not.toHaveText("Загрузка…", { timeout: 25_000 });

    await owner.getByLabel("Идентификатор Persona").fill("rt1-local-canary-persona");
    await owner.getByRole("button", { name: "Создать Persona" }).click();
    const answers = ["Слушаю внимательно", "Говорю спокойно", "Отвечаю кратко"];
    for (const [index, answer] of answers.entries()) {
      await owner.getByPlaceholder("Ответьте своими словами").fill(answer);
      await owner.getByRole("button", { name: "Сохранить ответ" }).click();
      await expect(owner.locator("#persona-progress")).toContainText(`${index + 1}/${answers.length}`);
    }
    await owner.getByRole("button", { name: "Перейти к проверке" }).click();
    const accept = owner.getByRole("button", { name: "Подтвердить без изменений" });
    await expect(accept).toHaveCount(answers.length);
    for (let index = 0; index < answers.length; index += 1) {
      await accept.first().click();
    }
    await owner.getByRole("button", { name: "Подтвердить Persona" }).click();
    await expect(owner.locator("#persona-progress")).toContainText("версия 2");

    // Publication is issued only after canonical server-side Persona review.
    await owner.goto("/rt1");
    await expect(owner.getByRole("heading", { name: "Persona · RT1" })).toBeVisible();
    await owner.locator("#provision").click();
    await expect(owner.locator("#status")).toContainText("Кабинет создан");
    const ownerToken = await owner.locator("#owner-token").inputValue();
    expect(ownerToken.length).toBeGreaterThan(30);
    await owner.locator("#persona-version").fill("2");
    await owner.locator("#publish").click();
    await expect(owner.locator("#status")).toContainText("Persona опубликована");
    const invite = await owner.locator("#invite-token").inputValue();
    expect(invite.length).toBeGreaterThan(30);

    // Distinct browser context, no shared cookie or owner identity.
    await visitor.goto("/rt1");
    await visitor.locator("#visitor-token").fill(invite);
    await visitor.locator("#consent").check();
    await visitor.locator("#join").click();
    await expect(visitor.locator("#status")).toContainText("Звук и видео подключены", {
      timeout: 40_000,
    });

    // This is actual aiortc sender -> Chromium RTP, not a mocked PeerConnection.
    await expect.poll(
      () => visitor.evaluate(() => (document.getElementById("video") as HTMLVideoElement).videoWidth),
      { timeout: 40_000 },
    ).toBeGreaterThan(0);
    const streams = await visitor.evaluate(() => {
      const video = document.getElementById("video") as HTMLVideoElement;
      const audio = document.getElementById("audio") as HTMLAudioElement;
      return {
        videoTracks: (video.srcObject as MediaStream | null)?.getVideoTracks().length ?? 0,
        audioTracks: (audio.srcObject as MediaStream | null)?.getAudioTracks().length ?? 0,
      };
    });
    expect(streams.videoTracks).toBeGreaterThan(0);
    expect(streams.audioTracks).toBeGreaterThan(0);

    await owner.locator("#pause").click();
    await expect(owner.locator("#status")).toContainText("Публикация отозвана");
    await expect(visitor.locator("#status")).toContainText(
      "Звонок завершён, остановка аудио и видео подтверждена",
      { timeout: 25_000 },
    );
    await expect(visitor.locator("#leave")).toBeDisabled();
    const ended = await visitor.evaluate(() => ({
      video: (document.getElementById("video") as HTMLVideoElement).srcObject === null,
      audio: (document.getElementById("audio") as HTMLAudioElement).srcObject === null,
    }));
    expect(ended).toEqual({ video: true, audio: true });
  } finally {
    await visitorContext.close();
    await ownerContext.close();
  }
});
