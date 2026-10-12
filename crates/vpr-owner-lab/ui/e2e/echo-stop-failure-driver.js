// Fail-closed STOP on a REAL Rust / Echo session, with no browser media
// publisher. Control-plane barriers use only the independent fixture mailbox,
// never Playwright page RPC after navigation.
(() => {
  const mailbox = "/__journey/report/expressive";
  const control = (id) => {
    const element = document.getElementById(id);
    if (!element) throw new Error("ECHO_STOP_CONTROL_MISSING:" + id);
    return element;
  };
  const poll = async (predicate, phase, timeout = 30000) => {
    const start = performance.now();
    while (performance.now() - start < timeout) {
      if (await predicate()) return;
      await new Promise((resolve) => setTimeout(resolve, 40));
    }
    throw new Error("ECHO_STOP_FAILURE_TIMEOUT:" + phase);
  };
  const report = async (event) => {
    const response = await fetch(mailbox, {
      method: "POST", headers: { "content-type": "text/plain;charset=UTF-8" },
      body: JSON.stringify(event), cache: "no-store",
    });
    if (!response.ok) throw new Error("ECHO_STOP_REPORT_FAILED");
  };
  const run = async () => {
    try {
      await poll(() => document.documentElement.dataset.vprProviderAutoConnect === "clicked", "auto-consent");
      await poll(() => {
        const status = control("status").textContent || "";
        return status.includes("LiveKit согласован") || status.includes("Аватар приветствует вас");
      }, "connected");
      const stop = control("interrupt");
      await poll(() => !stop.disabled, "stop-authorized");
      await report({ phase: "private-echo-viewer-ready" });
      // Controller verifies a real, authorized server-side Echo publish
      // before arming an injected HTTP failure for the next user STOP.
      await poll(async () => {
        const response = await fetch(mailbox, { cache: "no-store" });
        if (!response.ok) return false;
        const snapshot = await response.json();
        return snapshot.events?.some((event) => event.phase === "private-echo-audio-published") === true;
      }, "authorized-server-audio-published");
      if (window.__vprLiveKitCommands.some((command) =>
        command.topic === "did.interrupt" || command.topic === "did.speak")) {
        throw new Error("ECHO_STOP_PRIVATE_SENDER_EXPOSED_BEFORE_CLICK");
      }
      stop.click();
      if (!stop.disabled || !(stop.textContent || "").includes("Останавливаю")) {
        throw new Error("ECHO_STOP_CLICK_NOT_ACKNOWLEDGED");
      }
      if (!control("avatar-audio").muted || !control("avatar").muted) {
        throw new Error("ECHO_STOP_NOT_LOCALLY_MUTED");
      }
      stop.click();
      await poll(() =>
        control("status").textContent.includes("PROVIDER_STOP_UNCONFIRMED_SESSION_REVOKED"),
        "canonical-revoked-after-stop-denial",
      );
      if (!stop.disabled || window.__vprExpressiveRoomDisconnectCount < 1) {
        throw new Error("ECHO_STOP_DENIAL_LEFT_VIEWER_CONNECTED");
      }
      if (window.__vprLiveKitCommands.some((command) =>
        command.topic === "did.interrupt" || command.topic === "did.speak")) {
        throw new Error("ECHO_STOP_DENIAL_EXPOSED_BROWSER_PUBLISHER");
      }
      await report({ status: "ok", route: "real-pcm-echo-stop-denial-revoked" });
    } catch (error) {
      await report({ status: "failed", error: error instanceof Error ? error.message : String(error) })
        .catch(() => undefined);
    }
  };
  if (document.readyState === "loading")
    document.addEventListener("DOMContentLoaded", () => void run(), { once: true });
  else void run();
})();
