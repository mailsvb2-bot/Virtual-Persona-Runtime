(() => {
  const waitFor = async (predicate, phase, timeout = 45000) => {
    const start = performance.now();
    while (performance.now() - start < timeout) {
      if (await predicate()) return;
      await new Promise((resolve) => setTimeout(resolve, 30));
    }
    throw new Error("ECHO_OWNER_VOICE_TIMEOUT:" + phase);
  };
  const control = (id) => {
    const value = document.getElementById(id);
    if (!value) throw new Error("ECHO_OWNER_CONTROL_MISSING:" + id);
    return value;
  };
  const report = async (value) => {
    const response = await fetch("/__journey/report/expressive", {
      method: "POST",
      headers: { "content-type": "text/plain;charset=UTF-8" },
      body: JSON.stringify(value),
      cache: "no-store",
    });
    if (!response.ok) throw new Error("ECHO_OWNER_REPORT_FAILED");
  };
  const run = async () => {
    try {
      await waitFor(() => document.documentElement.dataset.vprProviderAutoConnect === "clicked", "consent");
      await waitFor(() => {
        const status = control("status").textContent || "";
        // A consented, TTS-capable server now greets after its first video frame.
        // Test the actual real-voice handoff, rather than assume the old
        // transient "LiveKit согласован" label remains visible forever.
        return status.includes("LiveKit согласован") || status.includes("Аватар приветствует вас");
      }, "connected");
      const voice = control("voice");
      await waitFor(() => !voice.disabled, "voice-ready");
      voice.click();
      await waitFor(() => (voice.textContent || "").includes("Остановить и отправить"), "recording");
      voice.click();
      await waitFor(() => (control("status").textContent || "").includes("Вы: Привет из браузера")
        && (control("status").textContent || "").includes("Ответ:"), "reply");
      // Click the real control; the publisher stays private to Rust/Python.
      const stop = control("interrupt");
      await waitFor(() => !stop.disabled, "server-echo-stop-available");
      if (window.__vprLiveKitCommands.some((c) =>
        c.topic === "did.interrupt" || c.topic === "did.speak")) {
        throw new Error("ECHO_BROWSER_PUBLISHER_LEAK");
      }
      stop.click();
      if (!stop.disabled || !(stop.textContent || "").includes("Останавливаю")) {
        throw new Error("ECHO_STOP_CLICK_NOT_ACKNOWLEDGED");
      }
      if (!control("avatar-audio").muted || !control("avatar").muted) {
        throw new Error("ECHO_STOP_NOT_MUTED_IMMEDIATELY");
      }
      stop.click();
      if (window.__vprExpectedEchoStopFailure === true) {
        await waitFor(() =>
          control("status").textContent.includes("PROVIDER_STOP_UNCONFIRMED_SESSION_REVOKED"),
          "server-stop-failed-canonical-revoked",
        );
        if (!stop.disabled || window.__vprExpressiveRoomDisconnectCount < 1) {
          throw new Error("ECHO_STOP_FAILURE_DID_NOT_CLOSE_VIEWER");
        }
        if (window.__vprLiveKitCommands.some((c) =>
          c.topic === "did.interrupt" || c.topic === "did.speak")) {
          throw new Error("ECHO_STOP_FAILURE_EXPOSED_PRIVATE_SENDER");
        }
        await report({ status: "ok", route: "browser-stop-failure-revoked" });
        return;
      }
      await waitFor(() => !stop.disabled, "server-echo-stop-acknowledged");
      if (!control("status").textContent.includes("Ответ остановлен")) {
        throw new Error("ECHO_STOP_STATUS_NOT_CONFIRMED");
      }
      if (window.__vprLiveKitCommands.some((c) =>
        c.topic === "did.interrupt" || c.topic === "did.speak")) {
        throw new Error("ECHO_BROWSER_STOP_LEAK");
      }
      await report({ status: "ok", route: "browser-to-server-private-echo-stop" });
    } catch (error) {
      await report({ status: "failed", error: error instanceof Error ? error.message : String(error) }).catch(() => {});
    }
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void run(), { once: true });
  } else void run();
})();
