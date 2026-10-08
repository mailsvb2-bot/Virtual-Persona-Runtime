(() => {
  const mailbox = "/__journey/report/expressive";
  const sleep = (ms) => new Promise((resolve) => window.setTimeout(resolve, ms));
  const waitFor = async (check, label, timeout = 30_000) => {
    const start = performance.now();
    while (performance.now() - start < timeout) {
      if (await check()) return;
      await sleep(20);
    }
    throw new Error("CROSS_TAB_REVOKE_TIMEOUT:" + label);
  };
  const get = (id) => {
    const result = document.getElementById(id);
    if (!result) throw new Error("CONTROL_MISSING:" + id);
    return result;
  };
  const statusText = () => get("status").textContent ?? "";
  const backend = async (path) => {
    const r = await fetch(path, { cache: "no-store" });
    if (!r.ok) throw new Error("BACKEND_READ_FAILED:" + path);
    return r.json();
  };
  const writeReport = async (data) => {
    const r = await fetch(mailbox, {
      method: "POST",
      headers: { "content-type": "text/plain;charset=UTF-8" },
      body: JSON.stringify(data),
      cache: "no-store",
    });
    if (!r.ok) throw new Error("REPORT_FAILED");
  };
  const run = async () => {
    try {
      await waitFor(() => statusText().includes("LiveKit согласован"), "connected");
      const voice = get("voice");
      await waitFor(() => !voice.disabled, "voice-ready");
      voice.click();
      await waitFor(() => voice.textContent.includes("Остановить и отправить"), "recording");
      voice.click();
      await waitFor(
        () => (window.__vprLiveKitCommands ?? []).some((c) => c.topic === "did.speak")
          && window.__vprExpressiveRemoteSpeech === true,
        "spoken-output-published",
      );
      const before = window.__vprLiveKitCommands.filter((c) => c.topic === "did.speak").length;
      // Controller now opens a genuinely separate same-origin tab only after
      // this checkpoint, so the new tab must discover the EXISTING active
      // canonical session without starting a second one.
      await writeReport({ kind: "phase", phase: "owner-speech-published" });
      await waitFor(
        () => (window.__vprExpressiveRoomDisconnectCount ?? 0) > 0,
        "other-tab-transport-closed",
        45_000,
      );
      if (!voice.disabled || !get("resume-answer-row").hidden) {
        throw new Error("CROSS_TAB_MEDIA_CONTROLS_STILL_OPEN");
      }
      await waitFor(async () => (await backend("/api/status")).session_state === "revoked", "revoked");
      await sleep(250);
      if (window.__vprLiveKitCommands.filter((c) => c.topic === "did.speak").length !== before) {
        throw new Error("SPEECH_AFTER_OTHER_TAB_REVOCATION");
      }
      const close = get("close");
      await waitFor(() => !close.disabled, "close-enabled");
      close.click();
      await waitFor(
        () => statusText().includes("Сессия закрыта. Evidence snapshot сохранён."),
        "evidence-exported",
      );
      const ended = await backend("/api/status");
      if (ended.session_state !== "closed" || ended.avatar_open !== false) {
        throw new Error("UNSAFE_TERMINAL_STATE");
      }
      const previousSession = (await backend("/api/evidence/session")).session_sequence;
      await waitFor(() => !get("connect").disabled, "new-session-available");
      get("connect").click();
      await waitFor(
        () => statusText().includes("LiveKit согласован"),
        "second-session-connected",
      );
      const currentSession = (await backend("/api/evidence/session")).session_sequence;
      if (!Number.isSafeInteger(previousSession)
          || !Number.isSafeInteger(currentSession)
          || currentSession <= previousSession) {
        throw new Error("CANONICAL_SESSION_SEQUENCE_DID_NOT_ADVANCE");
      }
      // The delayed unload/close of an earlier tab must not terminate this
      // freshly connected user session. This is tested through real HTTP.
      const csrf = (await backend("/api/bootstrap")).csrf_token;
      const staleClose = await fetch("/api/session/close", {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-VPR-CSRF": csrf },
        body: JSON.stringify({ expected_session_sequence: previousSession }),
        credentials: "same-origin",
        cache: "no-store",
      });
      if (staleClose.status !== 409) {
        throw new Error("STALE_TAB_CLOSE_NOT_REJECTED:" + staleClose.status);
      }
      const stillActive = await backend("/api/status");
      if (stillActive.session_state !== "active" || stillActive.avatar_open !== true) {
        throw new Error("STALE_TAB_CLOSE_KILLED_NEW_SESSION");
      }
      close.click();
      await waitFor(
        () => statusText().includes("Сессия закрыта. Evidence snapshot сохранён."),
        "second-session-evidence-exported",
      );
      await writeReport({
        status: "ok",
        session: "closed",
        cross_tab_fenced: true,
        no_late_speech: true,
        evidence_exported: true,
        stale_tab_close_denied: true,
        subsequent_session_survived: true,
      });
    } catch (error) {
      const terminal = await backend("/api/status").catch(() => null);
      await writeReport({
        status: "failed",
        error: (error instanceof Error ? error.message : String(error))
          + "@state:" + JSON.stringify({
            runtime: terminal?.session_state,
            avatarOpen: terminal?.avatar_open,
            status: statusText().slice(0, 130),
            disconnectCount: window.__vprExpressiveRoomDisconnectCount ?? 0,
          }),
      }).catch(() => undefined);
    }
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void run(), { once: true });
  } else void run();
})();
