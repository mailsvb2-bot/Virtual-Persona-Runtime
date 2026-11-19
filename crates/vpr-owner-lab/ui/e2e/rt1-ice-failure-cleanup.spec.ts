import { expect, test } from "@playwright/test";

// Browser-only failure injection. No mock result is treated as proof of
// real RTP, CUDA rendering, or remote STOP; the real server is covered by
// the separate owner/visitor voice and transport suites.
test("failed ICE gathering closes visitor and never submits a stale SDP answer", async ({ page }) => {
  let answers = 0;
  let closes = 0;
  await page.addInitScript(() => {
    class FailedIcePeer extends EventTarget {
      iceGatheringState = "gathering";
      connectionState = "new";
      signalingState = "stable";
      localDescription: RTCSessionDescriptionInit | null = null;
      ontrack: ((event: RTCTrackEvent) => void) | null = null;
      onconnectionstatechange: (() => void) | null = null;
      onicecandidate: ((event: RTCPeerConnectionIceEvent) => void) | null = null;
      async setRemoteDescription(_description: RTCSessionDescriptionInit) {}
      async createAnswer() { return { type: "answer", sdp: "v=0\\r\\n" }; }
      async setLocalDescription(description: RTCSessionDescriptionInit) {
        this.localDescription = description;
        setTimeout(() => {
          if (this.connectionState !== "new") return;
          this.connectionState = "failed";
          this.dispatchEvent(new Event("connectionstatechange"));
          this.onconnectionstatechange?.();
        }, 20);
      }
      close() {
        this.connectionState = "closed";
        this.signalingState = "closed";
      }
    }
    Object.defineProperty(window, "RTCPeerConnection", {
      configurable: true, writable: true, value: FailedIcePeer
    });
  });
  await page.route("**/api/rt1/visitor/start", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        session_id: "failure-test-session",
        voice_enabled: true,
        signal: { transport: { kind: "web_rtc", offer: { type: "offer", sdp: "v=0\\r\\n" } } }
      }),
    });
  });
  await page.route("**/api/rt1/visitor/answer", async (route) => {
    answers += 1;
    await route.fulfill({ status: 409, contentType: "application/json", body: '{"code":"AUTH_REVOKED"}' });
  });
  await page.route("**/api/rt1/visitor/close", async (route) => {
    closes += 1;
    await route.fulfill({
      status: 200, contentType: "application/json",
      body: '{"remote_stop_confirmed":true}'
    });
  });
  await page.goto("/rt1/visit", { waitUntil: "domcontentloaded" });
  await expect(page.locator("#status")).toHaveAttribute("data-rt1-ready", "true");
  await page.locator("#visitor-token").fill("a".repeat(64));
  await page.locator("#consent").check();
  await page.locator("#join").click();
  await expect.poll(() => closes, { timeout: 5_000 }).toBeGreaterThan(0);
  await expect(page.locator("#leave")).toBeDisabled();
  await expect(page.locator("#join")).toBeEnabled();
  expect(answers, "failed ICE must never reach server SDP answer").toBe(0);
});
