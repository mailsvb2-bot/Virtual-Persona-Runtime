(() => {
  const providerUrl = "http://127.0.0.1:18790";

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
    const response = await fetch(`${providerUrl}/__browser-journey`, {
      method: "POST",
      headers: { "content-type": "text/plain;charset=UTF-8" },
      body: JSON.stringify(update),
      mode: "cors",
      cache: "no-store",
    });
    if (!response.ok) throw new Error(`VOICE_JOURNEY_CHECKPOINT_HTTP_${response.status}`);
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
    setMessage(input);
    byId("speak").click();
    await waitForStatus(expectedStatus);
  };

  const recordVoiceTurn = async (expectedReply) => {
    const voice = byId("voice");
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
    await postJourney({ stage: "running", error: null });

    await waitForStatus("WebRTC согласован");
    await recordTextTurn("Текстовый вопрос владельца", "Ответ: Текстовый ответ владельцу");
    await recordVoiceTurn("Голосовой ответ владельцу");

    const interrupt = byId("interrupt");
    await waitFor(() => interrupt.disabled === false, "interrupt-enabled");
    interrupt.click();
    await waitForEvidence(
      (snapshot) => snapshot.media_events?.some((event) => event.kind === "interruption_stopped"),
      "interruption-stopped",
    );

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

    await recordTextTurn("Спровоцируй отказ провайдера", "PROVIDER_UNAVAILABLE");
    await recordTextTurn("Восстановление после отказа", "Ответ: Ответ после восстановления");

    const ownerEvidence = await fetchEvidence();
    byId("close").click();
    await waitForStatus("Сессия закрыта");

    const audience = byId("session-audience");
    audience.value = "visitor";
    audience.dispatchEvent(new Event("change", { bubbles: true }));
    await waitForConnectReady();
    byId("consent").checked = true;
    byId("connect").click();
    await waitForStatus("Visitor-сессия WebRTC согласована");

    await recordTextTurn("Текстовый вопрос visitor", "Ответ: Текстовый ответ visitor");
    await recordVoiceTurn("В visitor scope нет подтверждённых данных владельца");
    const visitorEvidence = await fetchEvidence();

    byId("revoke").click();
    await waitForStatus("Доступ отозван");
    byId("close").click();
    await waitForStatus("Сессия закрыта");

    await postJourney({
      stage: "complete",
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
        await postJourney({
          stage: "failed",
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
