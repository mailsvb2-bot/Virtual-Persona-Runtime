// Real Chromium RTCPeerConnection and local aiortc RTP with a synthetic renderer/provider fixture; never D-ID evidence.
import { expect, test } from "@playwright/test";

const ownerLabUrl = "http://127.0.0.1:18787";

test("separate owner and visitor contexts receive local aiortc RTP; revoke rejects late signaling (synthetic renderer; not D-ID)", async ({ browser }) => {
  const ownerContext = await browser.newContext();
  const visitorContext = await browser.newContext();
  const owner = await ownerContext.newPage();
  const visitor = await visitorContext.newPage();
  const visitorRequest = visitorContext.request;
  // Observe the actual receiving RTCPeerConnection without mocking the
  // constructor, tracks, receiver, signaling, or underlying RTP transport.
  await visitor.addInitScript(() => {
    const original = RTCPeerConnection.prototype.setRemoteDescription;
    RTCPeerConnection.prototype.setRemoteDescription = function (description) {
      (window as Window & { __rt1ReceivingPeer?: RTCPeerConnection }).__rt1ReceivingPeer = this;
      return original.call(this, description);
    };
  });
  let visitorSessionId: string | undefined;
  let originalAnswerSdp: string | undefined;
  let originalIce: { candidate: string; sdpMid?: string | null; sdpMLineIndex?: number | null } | undefined;
  visitor.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path !== "/api/rt1/visitor/answer" && path !== "/api/rt1/visitor/ice") return;
    const payload = request.postDataJSON() as { sdp?: unknown; candidate?: unknown; sdpMid?: unknown; sdpMLineIndex?: unknown };
    if (path === "/api/rt1/visitor/answer" && typeof payload.sdp === "string") originalAnswerSdp = payload.sdp;
    if (path === "/api/rt1/visitor/ice" && typeof payload.candidate === "string") {
      originalIce = {
        candidate: payload.candidate,
        sdpMid: typeof payload.sdpMid === "string" ? payload.sdpMid : null,
        sdpMLineIndex: typeof payload.sdpMLineIndex === "number" ? payload.sdpMLineIndex : null,
      };
    }
  });
  visitor.on("response", async (response) => {
    if (new URL(response.url()).pathname !== "/api/rt1/visitor/start" || !response.ok()) return;
    const body = await response.json();
    visitorSessionId = String(body.session_id);
  });

  try {
    await owner.goto(ownerLabUrl + "/", { waitUntil: "domcontentloaded", timeout: 30_000 });
    await expect(owner.locator("#workflow-state")).toContainText("Ожидает проверки владельцем", { timeout: 25_000 });

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
    await owner.goto(ownerLabUrl + "/rt1");
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
    await visitor.goto(ownerLabUrl + "/rt1");
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

    // Require inbound RTP packets on BOTH tracks, not just negotiated SDP or a connected state.
    await expect.poll(async () => visitor.evaluate(async () => {
      const receivingPeer = (window as Window & { __rt1ReceivingPeer?: RTCPeerConnection }).__rt1ReceivingPeer;
      if (!receivingPeer) return false;
      const stats = await receivingPeer.getStats();
      let audioPackets = 0;
      let videoPackets = 0;
      stats.forEach((report) => {
        if (report.type !== "inbound-rtp") return;
        const kind = report.kind ?? report.mediaType;
        if (kind === "audio") audioPackets += Number(report.packetsReceived ?? 0);
        if (kind === "video") videoPackets += Number(report.packetsReceived ?? 0);
      });
      return audioPackets > 0 && videoPackets > 0;
    }), { timeout: 30_000, message: "Chromium must receive actual audio and video RTP packets" }).toBe(true);

    // The visitor has a separate cookie jar and no owner bearer token.
    const visitorBootstrap = await visitorRequest.get(ownerLabUrl + "/api/bootstrap");
    expect(visitorBootstrap.ok()).toBeTruthy();
    const visitorCsrf = String((await visitorBootstrap.json()).csrf_token);
    const visitorHeaders = {
      "content-type": "application/json",
      "x-vpr-csrf": visitorCsrf,
      origin: new URL(visitor.url()).origin,
    };
    const forgedPause = await visitorRequest.post(ownerLabUrl + "/api/rt1/owner/pause", {
      headers: visitorHeaders,
      data: { owner_token: "forged-owner-token" },
    });
    expect(forgedPause.status()).toBe(403);
    expect((await forgedPause.json()).code).toBe("RT1_OWNER_ACTION_DENIED");

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

    // The same real browser-facing HTTP boundary must reject stale sessions
    // and publication tokens after owner revocation. STOP alone is not enough:
    // no late SDP or new visitor session may resurrect the media sender.
    const staleJoin = await visitorRequest.post(ownerLabUrl + "/api/rt1/visitor/start", {
      headers: visitorHeaders,
      data: { visitor_token: invite, consent: true },
    });
    expect(staleJoin.status(), "revoked publication must deny allocation, not fail for an unrelated reason").toBe(409);
    const staleJoinBody = await staleJoin.json();
    expect(staleJoinBody).not.toHaveProperty("session_id");
    expect(visitorSessionId, "the browser must have received a real session id").toBeTruthy();
    const activeAfterRevoke = await visitorRequest.post(ownerLabUrl + "/api/rt1/visitor/status", {
      headers: visitorHeaders,
      data: { session_id: visitorSessionId },
    });
    if (activeAfterRevoke.ok()) {
      expect((await activeAfterRevoke.json()).active).toBe(false);
    } else {
      expect(activeAfterRevoke.status()).toBe(404);
    }
    // Replay the actual Chromium-generated SDP, not malformed placeholder text:
    // otherwise a validation error could falsely satisfy the revoke regression.
    expect(originalAnswerSdp, "Chromium must have sent a valid SDP answer").toMatch(/^v=0/);
    const staleAnswer = await visitorRequest.post(ownerLabUrl + "/api/rt1/visitor/answer", {
      headers: visitorHeaders,
      data: { session_id: visitorSessionId, sdp: originalAnswerSdp },
    });
    // A confirmed STOP may remove the session (404); a retained revoked
    // session must reject the answer (409). Neither may reach the sender.
    expect([404, 409], "revoked visitor must reject a valid late SDP answer").toContain(staleAnswer.status());
    if (originalIce) {
      const staleIce = await visitorRequest.post(ownerLabUrl + "/api/rt1/visitor/ice", {
        headers: visitorHeaders,
        data: { session_id: visitorSessionId, ...originalIce },
      });
      expect([404, 409], "revoked visitor must reject a valid late ICE candidate").toContain(staleIce.status());
    }
  } finally {
    await visitorContext.close();
    await ownerContext.close();
  }
});
