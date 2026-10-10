(() => {
  const mailbox = "/__journey/report/expressive";
  const sessionRequestOrder = [];
  const nativeFetch = window.fetch.bind(window);
  window.fetch = (input, init) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input?.url ?? "";
    const method = (init?.method ?? (typeof input === "object" ? input?.method : undefined) ?? "GET").toUpperCase();
    const pathname = new URL(url, location.href).pathname;
    if (method === "POST" && (pathname === "/api/session/revoke" || pathname === "/api/session/close")) {
      sessionRequestOrder.push(pathname);
    }
    return nativeFetch(input, init);
  };
  const sleep = (ms) => new Promise((resolve) => window.setTimeout(resolve, ms));
  const waitFor = async (predicate, phase, timeout = 25_000) => {
    const started = performance.now();
    while (performance.now() - started < timeout) {
      if (await predicate()) return;
      await sleep(20);
    }
    throw new Error("UNEXPECTED_LIVEKIT_DISCONNECT_TIMEOUT:" + phase);
  };
  const get = (id) => {
    const node = document.getElementById(id);
    if (!node) throw new Error("MISSING_CONTROL:" + id);
    return node;
  };
  const statusText = () => get("status").textContent ?? "";
  const commands = () => window.__vprLiveKitCommands ?? [];
  const backendState = async () => {
    const result = await fetch("/api/status", { cache: "no-store" });
    return result.ok ? result.json() : null;
  };
  const report = async (value) => {
    const response = await fetch(mailbox, {
      method: "POST",
      headers: { "content-type": "text/plain;charset=UTF-8" },
      body: JSON.stringify(value),
      cache: "no-store",
    });
    if (!response.ok) throw new Error("DISCONNECT_JOURNEY_REPORT_FAILED");
  };
  const run = async () => {
    try {
      await waitFor(() => statusText().includes("LiveKit согласован"), "connected");
      if (document.documentElement.dataset.vprProviderAutoConnect !== "clicked") {
        throw new Error("AUTO_CONNECT_NOT_EXERCISED");
      }
      const voice = get("voice");
      await waitFor(() => !voice.disabled, "voice-ready");
      voice.click();
      await waitFor(() => voice.textContent.includes("Остановить и отправить"), "recording");
      voice.click();
      // Disconnect only AFTER the live speech command has escaped into the
      // synthetic provider. This is not the trivial pre-connect cleanup case.
      await waitFor(
        () => commands().some((command) => command.topic === "did.speak")
          && window.__vprExpressiveRemoteSpeech === true,
        "provider-speech-in-flight",
      );
      const priorSpeakCount = commands().filter((command) => command.topic === "did.speak").length;
      const disconnect = window.__vprExpressiveDisconnect;
      if (typeof disconnect !== "function") throw new Error("NO_PROVIDER_DISCONNECT_FIXTURE");
      disconnect();
      await waitFor(async () => (await backendState())?.session_state === "closed", "canonical-closed");
      await waitFor(() => statusText().includes("Сессия закрыта"), "user-visible-closed");
      if ((window.__vprExpressiveRoomDisconnectCount ?? 0) < 1) {
        throw new Error("CLIENT_PUBLISH_TRANSPORT_NOT_RELEASED");
      }
      const runtime = await backendState();
      if (runtime?.avatar_open !== false) {
        throw new Error("REMOTE_AVATAR_RESOURCE_NOT_CLOSED");
      }
      const revokeIndex = sessionRequestOrder.indexOf("/api/session/revoke");
      const closeIndex = sessionRequestOrder.indexOf("/api/session/close");
      if (revokeIndex < 0 || closeIndex <= revokeIndex) {
        throw new Error("CANONICAL_REVOKE_DID_NOT_PRECEDE_EVIDENCE_CLOSE:"
          + JSON.stringify(sessionRequestOrder));
      }
      if (!voice.disabled || !get("resume-answer-row").hidden) {
        throw new Error("POST_DISCONNECT_MEDIA_EGRESS_CONTROLS_OPEN");
      }
      await sleep(250);
      if (commands().filter((command) => command.topic === "did.speak").length !== priorSpeakCount) {
        throw new Error("QUEUED_SPEECH_ESCAPED_AFTER_PROVIDER_DISCONNECT");
      }
      await report({
        status: "ok",
        session: "closed",
        provider_resource_closed: true,
        browser_transport_closed: true,
        no_late_speak: true,
        revoke_before_close: true,
      });
    } catch (error) {
      const runtime = await backendState().catch(() => null);
      await report({
        status: "failed",
        error: (error instanceof Error ? error.message : String(error))
          + "@state:" + JSON.stringify({
            session: runtime?.session_state,
            avatarOpen: runtime?.avatar_open,
            visibleStatus: statusText().slice(0, 120),
            sentSpeak: commands().filter((command) => command.topic === "did.speak").length,
            roomDisconnectCount: window.__vprExpressiveRoomDisconnectCount ?? 0,
          }),
      }).catch(() => undefined);
    }
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void run(), { once: true });
  } else {
    void run();
  }
})();
