import { expect, test } from "@playwright/test";

test("LiveKit SDK executes from the same-origin locked vendor asset", async ({ page, request }) => {
  const asset = await request.get("/vendor/livekit-client.umd.min.js");
  expect(asset.ok()).toBeTruthy();
  expect(asset.headers()["content-type"]).toContain("text/javascript");

  await page.goto("/");
  await page.addScriptTag({ url: "/vendor/livekit-client.umd.min.js" });
  const roomType = await page.evaluate(
    () =>
      typeof (window as typeof window & {
        LivekitClient?: { Room?: unknown };
      }).LivekitClient?.Room,
  );
  expect(roomType).toBe("function");
});
