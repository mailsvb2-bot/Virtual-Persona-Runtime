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
      // The pre-navigation provider-bootstrap harness already ticks consent
      // and clicks Connect. A second click or wait for an enabled Connect
      // button races that harness and can deadlock after a successful connect.
      // Observe the real session outcome instead of taking control twice.
      await waitFor(() => statusText().includes("LiveKit согласован"), "connected");
      if (document.documentElement.dataset.vprProviderAutoConnect !== "clicked") {
        throw new Error("PROVIDER_AUTO_CONNECT_NOT_EXERCISED");
      }
      const voice = element("voice");
      const interrupt = element("interrupt");
      await waitFor(() => !voice.disabled, "voice-ready");
      voice.click();
      await waitFor(
        () => (voice.textContent ?? "").includes("Остановить и отправить"),
        "voice-recording",
      );
      voice.click();
      // A canonical in-flight STT/LLM turn can enable Interrupt even before a
      // single D-ID speak was sent. Require actual client/provider speech first,
      // otherwise we would test voice cancellation instead of failed STOP.
      await waitFor(() =>
        (window.__vprLiveKitCommands ?? []).some((command) => command.topic === "did.speak")
        && window.__vprExpressiveRemoteSpeech === true
        && !interrupt.disabled,
      "actual-provider-speech-before-stop-failure");
      // A real D-ID STOP could reject while the remote media stream keeps
      // playing. The browser must disconnect BEFORE invoking REST revoke.
      window.__vprExpressiveFailNextInterrupt = true;
      interrupt.click();
      await waitFor(
        () => statusText().includes("PROVIDER_STOP_UNCONFIRMED_SESSION_REVOKED")
          || statusText().includes("PROVIDER_STOP_UNCONFIRMED_CANONICAL_REVOKED_CLEANUP_PENDING"),
        "stop-failed-and-revoked",
      );
      if (window.__vprExpressiveFailNextInterrupt !== false) {
        throw new Error("DID_STOP_SEND_REJECTION_NOT_EXERCISED");
      }
      if ((window.__vprExpressiveRoomDisconnectCount ?? 0) < 1) {
        throw new Error("LIVEKIT_PUBLISH_TOKEN_NOT_DISCONNECTED");
      }
      const response = await fetch("/api/status", { cache: "no-store" });
      const runtime = response.ok ? await response.json() : null;
      if (!runtime || runtime.session_state !== "revoked") {
        throw new Error("CANONICAL_REVOKE_NOT_COMMITTED");
      }
      if (runtime.avatar_open !== false) {
        throw new Error("REMOTE_AVATAR_RESOURCE_STILL_OPEN");
      }
      if (!element("resume-answer-row").hidden || !voice.disabled) {
        throw new Error("REVOKED_SESSION_STILL_OFFERS_MEDIA_EGRESS");
      }
      await sendReport({
        status: "ok",
        session: "revoked",
        provider_resource_closed: true,
        stop_failed: true,
        stream_cleanup_pending: statusText().includes("CLEANUP_PENDING"),
      });
    } catch (error) {
      const state = await fetch("/api/status", { cache: "no-store" })
        .then((response) => response.ok ? response.json() : null)
        .catch(() => null);
      const commands = window.__vprLiveKitCommands ?? [];
      const diagnosis = {
        display: statusText().slice(0, 150),
        session: state?.session_state ?? "unavailable",
        sentSpeak: commands.filter((command) => command.topic === "did.speak").length,
        sentStop: commands.filter((command) => command.topic === "did.interrupt").length,
        stopFailurePending: window.__vprExpressiveFailNextInterrupt === true,
        disconnectedRooms: window.__vprExpressiveRoomDisconnectCount ?? 0,
        voiceDisabled: element("voice").disabled,
        interruptDisabled: element("interrupt").disabled,
      };
      await sendReport({
        status: "failed",
        error: (error instanceof Error ? error.message : String(error))
          + "@state:" + JSON.stringify(diagnosis),
      }).catch(() => undefined);
    }
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void run(), { once: true });
  } else {
    void run();
  }
})();
