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
      const snap = await backend("/api/evidence/session");
      const seq = snap.session_sequence;
      if (!Number.isSafeInteger(seq) || seq < 1) throw new Error("INVALID_SESSION_SEQUENCE");
      const before = window.__vprLiveKitCommands.filter((c) => c.topic === "did.speak").length;
      const otherTab = new BroadcastChannel("vpr.owner-lab.session-egress-fence.v1");
      // Simulate a separate same-origin tab beginning a revoke: emit the
      // best-effort early browser fence before making the canonical REST call.
      otherTab.postMessage({
        kind: "session-egress-revoked",
        evidence_session_sequence: seq,
      });
      await waitFor(
        () => (window.__vprExpressiveRoomDisconnectCount ?? 0) > 0,
        "other-tab-transport-closed",
      );
      if (!voice.disabled || !get("resume-answer-row").hidden) {
        throw new Error("CROSS_TAB_MEDIA_CONTROLS_STILL_OPEN");
      }
      const csrf = (await backend("/api/bootstrap")).csrf_token;
      const revoke = await fetch("/api/session/revoke", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "X-VPR-CSRF": csrf,
        },
        body: "{}",
        credentials: "same-origin",
        cache: "no-store",
      });
      if (!revoke.ok) throw new Error("CANONICAL_REVOKE_FAILED:" + revoke.status);
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
      otherTab.close();
      await writeReport({
        status: "ok",
        session: "closed",
        cross_tab_fenced: true,
        no_late_speech: true,
        evidence_exported: true,
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
