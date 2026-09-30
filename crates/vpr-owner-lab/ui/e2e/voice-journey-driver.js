(() => {
  const reportUrl = "/__journey/report/voice";

  const delay = (millis) => new Promise((resolve) => window.setTimeout(resolve, millis));

  const waitFor = async (predicate, label, timeoutMillis = 20_000) => {
    const deadline = performance.now() + timeoutMillis;
    while (performance.now() < deadline) {
      if (await predicate()) return;
      await delay(20);
    }
    throw new Error(`VOICE_JOURNEY_TIMEOUT:${label}`);
  };

  const byId = (id) => {
    const node = document.getElementById(id);
    if (!node) throw new Error(`VOICE_JOURNEY_MISSING:${id}`);
    return node;
  };

  const statusText = () => byId("status").textContent ?? "";

  const waitForStatus = (expected) =>
    waitFor(() => statusText().includes(expected), `status:${expected}`);

  const postJourney = async (update) => {
    await fetch(reportUrl, {
      method: "POST",
      headers: { "content-type": "text/plain;charset=UTF-8" },
      body: JSON.stringify(update),
      cache: "no-store",
    });
  };

  const fetchEvidence = async () => {
    const response = await fetch("/api/evidence/session", {
      credentials: "same-origin",
      cache: "no-store",
    });
    if (!response.ok) throw new Error(`VOICE_JOURNEY_EVIDENCE_HTTP_${response.status}`);
    return response.json();
  };

  const waitForEvidence = async (predicate, label) => {
    await waitFor(async () => {
      try {
        return predicate(await fetchEvidence());
      } catch {
        return false;
      }
    }, `evidence:${label}`);
  };

  const setMessage = (value) => {
    const message = byId("message");
    message.value = value;
    message.dispatchEvent(new Event("input", { bubbles: true }));
  };

  const recordTextTurn = async (input, expectedStatus) => {
    const speak = byId("speak");
    await waitFor(() => !speak.disabled, "text-send-enabled");
    setMessage(input);
    speak.click();
    await waitForStatus(expectedStatus);
  };

  const recordVoiceTurn = async (expectedReply) => {
    const voice = byId("voice");
    await waitFor(() => !voice.disabled, "voice-enabled");
    voice.click();
    await waitFor(
      () => voice.textContent === "Остановить и отправить",
      "voice-recording",
    );
    await waitFor(
      () => (byId("microphone-level-text").textContent ?? "").includes("RMS"),
      "microphone-sample",
    );
    voice.click();
    await waitForStatus(`Ответ: ${expectedReply}`);
    await waitForEvidence(
      (snapshot) =>
        snapshot.canonical_playback_proven === true
        && snapshot.av_sync_proven === true
        && snapshot.voice_attempts?.some(
          (attempt) => attempt.status === "completed" && attempt.canonical_playback_confirmed === true,
        ),
      "canonical-playback",
    );
  };

  const waitForConnectReady = async () => {
    const connect = byId("connect");
    await waitFor(() => connect.disabled === false, "connect-enabled");
  };

  const run = async () => {
    let phase = "driver-started";
    const checkpoint = async (nextPhase) => {
      phase = nextPhase;
      document.documentElement.dataset.vprVoiceJourneyPhase = phase;
      await postJourney({ phase });
    };
    document.documentElement.dataset.vprVoiceJourneyPhase = phase;
    await postJourney({ stage: "running", phase, error: null });

    await waitForStatus("WebRTC согласован");
    await checkpoint("owner-connected");
    await recordTextTurn("Текстовый вопрос владельца", "Ответ: Текстовый ответ владельцу");
    await checkpoint("owner-text-complete");
    await recordVoiceTurn("Голосовой ответ владельцу");
    await checkpoint("owner-playback-proven");

    const interrupt = byId("interrupt");
    await waitFor(() => interrupt.disabled === false, "interrupt-enabled");
    interrupt.click();
    await waitForEvidence(
      (snapshot) => snapshot.media_events?.some((event) => event.kind === "interruption_stopped"),
      "interruption-stopped",
    );
    await checkpoint("owner-interrupt-proven");

    const setPeerState = window.__vprSetPeerConnectionState;
    if (typeof setPeerState !== "function") throw new Error("VOICE_JOURNEY_PEER_CONTROL_MISSING");
    setPeerState("disconnected");
    await delay(40);
    setPeerState("connected");
    await waitForEvidence(
      (snapshot) => snapshot.media_events?.some(
        (event) => event.kind === "reconnect_restored" && event.elapsed_millis > 0,
      ),
      "reconnect-restored",
    );
    await checkpoint("owner-reconnect-proven");

    await recordTextTurn("Спровоцируй отказ провайдера", "PROVIDER_UNAVAILABLE");
    await recordTextTurn("Восстановление после отказа", "Ответ: Ответ после восстановления");
    await checkpoint("owner-recovery-complete");

    const ownerEvidence = await fetchEvidence();
    byId("close").click();
    await waitForStatus("Сессия закрыта");
    await checkpoint("owner-closed");

    const audience = byId("session-audience");
    audience.value = "visitor";
    audience.dispatchEvent(new Event("change", { bubbles: true }));
    await waitForConnectReady();
    byId("consent").checked = true;
    byId("connect").click();
    await waitForStatus("Visitor-сессия WebRTC согласована");
    await checkpoint("visitor-connected");

    await recordTextTurn("Текстовый вопрос visitor", "Ответ: Текстовый ответ visitor");
    await checkpoint("visitor-text-complete");
    await recordVoiceTurn("В visitor scope нет подтверждённых данных владельца");
    await checkpoint("visitor-playback-proven");
    const visitorEvidence = await fetchEvidence();

    byId("revoke").click();
    await waitForStatus("Доступ отозван");
    byId("close").click();
    await waitForStatus("Сессия закрыта");
    await checkpoint("visitor-closed");

    await postJourney({
      stage: "complete",
      phase: "complete",
      ownerEvidence,
      visitorEvidence,
      requestedMicrophones: window.__vprRequestedMicrophones ?? [],
      interruptPayloads: window.__vprInterruptPayloads ?? [],
    });
  };

  const start = () => {
    void run().catch(async (error) => {
      const message = error instanceof Error ? error.message : String(error);
      try {
        const current = document.documentElement.dataset.vprVoiceJourneyPhase ?? "unreported";
        await postJourney({
          stage: "failed",
          phase: current,
          error: message,
          requestedMicrophones: window.__vprRequestedMicrophones ?? [],
          interruptPayloads: window.__vprInterruptPayloads ?? [],
        });
      } catch {
        // The Playwright request-side timeout will expose a fixture transport failure.
      }
    });
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start, { once: true });
  } else {
    start();
  }
})();
