(() => {
  const mailbox = "/__journey/report/expressive";
  const sleep = (ms) => new Promise((resolve) => window.setTimeout(resolve, ms));
  const waitFor = async (predicate, label, limit = 25_000) => {
    const start = performance.now();
    while (performance.now() - start < limit) {
      if (await predicate()) return;
      await sleep(20);
    }
    throw new Error("PROVIDER_STOP_JOURNEY_TIMEOUT:" + label);
  };
  const element = (id) => {
    const node = document.getElementById(id);
    if (!node) throw new Error("CONTROL_MISSING:" + id);
    return node;
  };
  const statusText = () => element("status").textContent ?? "";
  const sendReport = async (result) => {
    const response = await fetch(mailbox, {
      method: "POST",
      headers: { "content-type": "text/plain;charset=UTF-8" },
      body: JSON.stringify(result),
      cache: "no-store",
    });
    if (!response.ok) throw new Error("JOURNEY_REPORT_FAILED");
  };
  const run = async () => {
    try {
      // An earlier full-journey test closed the old session but left a
      // reviewed Persona. Reconnect as that same owner without resetting it.
      const consent = element("consent");
      if (!consent.checked) consent.click();
      const connect = element("connect");
      await waitFor(() => !connect.disabled, "connect-enabled");
      connect.click();
      await waitFor(() => statusText().includes("LiveKit согласован"), "connected");
      const voice = element("voice");
      const interrupt = element("interrupt");
      await waitFor(() => !voice.disabled, "voice-ready");
      voice.click();
      await waitFor(
        () => (voice.textContent ?? "").includes("Остановить и отправить"),
        "voice-recording",
      );
      voice.click();
      await waitFor(() => !interrupt.disabled, "interrupt-enabled");
      // A real D-ID STOP could reject while the remote media stream keeps
      // playing. The browser must disconnect BEFORE invoking REST revoke.
      window.__vprExpressiveFailNextInterrupt = true;
      interrupt.click();
      await waitFor(
        () => statusText().includes("PROVIDER_STOP_UNCONFIRMED_SESSION_REVOKED"),
        "stop-failed-and-revoked",
      );
      if (window.__vprExpressiveFailNextInterrupt !== false) {
        throw new Error("DID_STOP_SEND_REJECTION_NOT_EXERCISED");
      }
      if ((window.__vprExpressiveRoomDisconnectCount ?? 0) < 1) {
        throw new Error("LIVEKIT_PUBLISH_TOKEN_NOT_DISCONNECTED");
      }
      const response = await fetch("/api/status", { cache: "no-store" });
      if (!response.ok || (await response.json()).session_state !== "revoked") {
        throw new Error("CANONICAL_REVOKE_NOT_COMMITTED");
      }
      if (!element("resume-answer-row").hidden || !voice.disabled) {
        throw new Error("REVOKED_SESSION_STILL_OFFERS_MEDIA_EGRESS");
      }
      await sendReport({ status: "ok", session: "revoked", stop_failed: true });
    } catch (error) {
      await sendReport({
        status: "failed",
        error: error instanceof Error ? error.message : String(error),
      }).catch(() => undefined);
    }
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void run(), { once: true });
  } else {
    void run();
  }
})();
