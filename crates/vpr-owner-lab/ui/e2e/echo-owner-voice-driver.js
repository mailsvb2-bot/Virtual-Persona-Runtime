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
      await waitFor(() => (control("status").textContent || "").includes("LiveKit согласован"), "connected");
      const voice = control("voice");
      await waitFor(() => !voice.disabled, "voice-ready");
      voice.click();
      await waitFor(() => (voice.textContent || "").includes("Остановить и отправить"), "recording");
      voice.click();
      await waitFor(() => (control("status").textContent || "").includes("Вы: Привет из браузера")
        && (control("status").textContent || "").includes("Ответ:"), "reply");
      await report({ status: "ok", route: "browser-microphone-to-server-echo" });
    } catch (error) {
      await report({ status: "failed", error: error instanceof Error ? error.message : String(error) }).catch(() => {});
    }
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void run(), { once: true });
  } else void run();
})();
