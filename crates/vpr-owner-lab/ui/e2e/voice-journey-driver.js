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
  const correctedOwnerPrinciple = "Проверяю факты перед ответом, никогда не выдумываю информацию";
  const canonicalPost = async (path, body, headers = {}) => {
    const bootstrap = await fetch("/api/bootstrap", { cache: "no-store" });
    if (!bootstrap.ok) throw new Error("OWNER_JOURNEY_BOOTSTRAP_UNAVAILABLE");
    const session = await bootstrap.json();
    if (!session.csrf_token) throw new Error("OWNER_JOURNEY_CSRF_MISSING");
    return fetch(path, {
      method: "POST", credentials: "same-origin", cache: "no-store",
      headers: {
        "content-type": "application/json",
        "x-vpr-csrf": session.csrf_token, ...headers,
      },
      body: JSON.stringify(body),
    });
  };

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
    const finishPlayback = window.__vprFinishSyntheticPlayback;
    if (typeof finishPlayback !== "function") {
      throw new Error("VOICE_JOURNEY_PLAYBACK_COMPLETION_CONTROL_MISSING");
    }
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
    await waitFor(() => !byId("interrupt").disabled, "playback-started");
    await waitForEvidence(
      (snapshot) => snapshot.media_events?.some(
        (event) => event.request_sequence === 1 && event.kind === "audio_started",
      ),
      "first-audio-before-playback-complete",
    );
    // Capture real progressing RTP while the simulated avatar is SPEAKING,
    // not after the fixture has stopped feeding audio packets.
    await waitForEvidence(
      (snapshot) => snapshot.av_sync_samples?.filter(
        (sample) => sample.request_sequence === 1
          && sample.reference === "web_rtc_estimated_playout_timestamp",
      ).length === 3,
      "advancing-rtp-av-sync-before-playback-complete",
    );
    finishPlayback();
    await waitForEvidence(
      (snapshot) =>
        snapshot.canonical_playback_proven === true
        && snapshot.av_sync_proven === true
        && snapshot.voice_attempts?.some(
          (attempt) => attempt.status === "completed" && attempt.canonical_playback_confirmed === true,
        )
        && snapshot.media_events?.some(
          (event) => event.kind === "playback_completed",
        ),
      "canonical-playback",
    );
  };

  const recordInterruptedVoiceTurn = async (expectedReply) => {
    const voice = byId("voice");
    await waitFor(() => !voice.disabled, "interrupt-turn-voice-enabled");
    voice.click();
    await waitFor(
      () => voice.textContent === "Остановить и отправить",
      "interrupt-turn-recording",
    );
    await waitFor(
      () => (byId("microphone-level-text").textContent ?? "").includes("RMS"),
      "interrupt-turn-microphone-sample",
    );
    voice.click();
    await waitForStatus(`Ответ: ${expectedReply}`);
    const beginPlayback = window.__vprBeginSyntheticPlayback;
    if (typeof beginPlayback !== "function") {
      throw new Error("VOICE_JOURNEY_PLAYBACK_START_CONTROL_MISSING");
    }
    // The deterministic fixture owns provider playback timing. Explicitly start the
    // second playback here so interruption evidence never depends on a polling race
    // between the final voice-event batch and the synthetic provider notification.
    beginPlayback();
    const interrupt = byId("interrupt");
    await waitFor(() => interrupt.disabled === false, "interrupt-enabled");
    await waitForEvidence(
      (snapshot) => snapshot.media_events?.some(
        (event) => event.request_sequence === 2 && event.kind === "audio_started",
      ),
      "interrupt-audio-started",
    );
    interrupt.click();
    await waitForEvidence(
      (snapshot) => snapshot.media_events?.some(
        (event) => event.request_sequence === 2 && event.kind === "interruption_stopped",
      ),
      "interruption-stopped",
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

    await recordInterruptedVoiceTurn("В visitor scope нет подтверждённых данных владельца");
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

    // Preserve the old-session evidence and transition through the REAL
    // owner correction endpoint. No in-flight media is rebound to a new version.
    const correction = await canonicalPost("/api/persona/claims/correct", {
      claim_id: "opinion-core-principle",
      statement: correctedOwnerPrinciple,
      kind: "opinion",
    });
    if (!correction.ok) throw new Error("OWNER_CORRECTION_HTTP_" + correction.status);
    const correctedStatus = await correction.json();
    if (correctedStatus.persona_version !== 3
        || correctedStatus.owner_context_state !== "reviewed") {
      throw new Error("OWNER_CORRECTION_VERSION_NOT_ADVANCED");
    }
    const reviewed = await canonicalPost("/api/persona/reviewed", {});
    if (!reviewed.ok) throw new Error("OWNER_REVIEWED_CONTEXT_UNAVAILABLE");
    const revisedProfile = await reviewed.json();
    const revisedClaim = (revisedProfile.claims ?? []).find(
      (claim) => claim.claim_id === "opinion-core-principle");
    if (revisedProfile.persona_version !== 3
        || revisedClaim?.statement !== correctedOwnerPrinciple
        || (revisedProfile.claims ?? []).some((claim) =>
          claim.statement === "Точность важнее уверенного выдумывания")) {
      throw new Error("STALE_OWNER_CLAIM_STILL_CURRENT");
    }
    await checkpoint("owner-correction-version3");

    // New owner session proves next actual LLM requests consume PersonaVersion 3.
    const audience = byId("session-audience");
    audience.value = "owner";
    audience.dispatchEvent(new Event("change", { bubbles: true }));
    await waitForConnectReady();
    byId("consent").checked = true;
    byId("connect").click();
    await waitForStatus("WebRTC согласован");
    await checkpoint("corrected-owner-connected");
    await recordTextTurn("Вопрос после исправления владельцем", "Ответ: Исправленное мнение учтено");
    await checkpoint("corrected-owner-text-complete");
    await recordVoiceTurn("В visitor scope нет подтверждённых данных владельца");
    await checkpoint("corrected-owner-voice-complete");
    const correctedOwnerEvidence = await fetchEvidence();
    byId("close").click();
    await waitForStatus("Сессия закрыта");
    await checkpoint("corrected-owner-closed");

    audience.value = "visitor";
    audience.dispatchEvent(new Event("change", { bubbles: true }));
    await waitForConnectReady();
    byId("consent").checked = true;
    byId("connect").click();
    await waitForStatus("Visitor-сессия WebRTC согласована");
    await checkpoint("visitor-connected");
    const mutation = await canonicalPost("/api/persona/claims/correct", {
      claim_id: "opinion-core-principle",
      statement: "ПОСЕТИТЕЛЬ НЕ МОЖЕТ МЕНЯТЬ ЧАСТНЫЕ ФАКТЫ",
      kind: "opinion",
    });
    const mutationResult = await mutation.json();
    if (mutation.status !== 403 || mutationResult.code !== "AUTH_SCOPE_DENIED") {
      throw new Error("VISITOR_PERSONA_MUTATION_NOT_DENIED");
    }
    const forbiddenRead = await canonicalPost("/api/persona/reviewed", {});
    if (forbiddenRead.status !== 403) throw new Error("VISITOR_READ_OWNER_CLAIMS_ALLOWED");
    await checkpoint("visitor-owner-mutation-denied");

    await recordTextTurn("Текстовый вопрос visitor", "Ответ: Текстовый ответ visitor");
    await checkpoint("visitor-text-complete");
    await recordVoiceTurn("В visitor scope нет подтверждённых данных владельца");
    await checkpoint("visitor-playback-proven");
    const visitorEvidence = await fetchEvidence();

    byId("revoke").click();
    await waitForStatus("Доступ отозван");
    const afterRevoke = await canonicalPost(
      "/api/text/turn",
      { text: "Запретить запрос после отзыва доступа" },
      { "x-vpr-evidence-request": "99" },
    );
    const revokedBody = await afterRevoke.json();
    if (afterRevoke.status !== 409
        || revokedBody.code !== "INVALID_STATE_TRANSITION") {
      throw new Error("REVOKED_SESSION_ACCEPTED_NEW_LLM");
    }
    await checkpoint("revoke-protected-request-denied");
    byId("close").click();
    await waitForStatus("Сессия закрыта");
    await checkpoint("visitor-closed");

    await postJourney({
      stage: "complete",
      phase: "complete",
      ownerEvidence,
      correctedOwnerEvidence,
      correctedPersonaVersion: 3,
      visitorMutationDenied: true,
      postRevokeLlmDenied: true,
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
