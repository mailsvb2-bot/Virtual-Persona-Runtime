(() => {
  const reportUrl = "/__voice_journey_report";
  const sleep = (millis) => new Promise((resolve) => window.setTimeout(resolve, millis));

  const waitFor = async (predicate, label, timeoutMillis = 12_000) => {
    const started = performance.now();
    while (performance.now() - started < timeoutMillis) {
      try {
        if (await predicate()) return;
      } catch {
        // Retry while the production UI is advancing through its real async lifecycle.
      }
      await sleep(20);
    }
    throw new Error(`VOICE_JOURNEY_TIMEOUT:${label}`);
  };

  const element = (id, Constructor) => {
    const value = document.getElementById(id);
    if (!(value instanceof Constructor)) throw new Error(`VOICE_JOURNEY_CONTROL_MISSING:${id}`);
    return value;
  };

  const statusText = () => element("status", HTMLElement).textContent ?? "";
  const waitStatus = (expected) => waitFor(
    () => statusText().includes(expected),
    `status:${expected}`,
  );

  const setText = (node, value) => {
    node.value = value;
    node.dispatchEvent(new Event("input", { bubbles: true }));
    node.dispatchEvent(new Event("change", { bubbles: true }));
  };

  const sessionEvidence = async () => {
    const response = await fetch("/api/evidence/session", {
      method: "GET",
      credentials: "same-origin",
      cache: "no-store",
    });
    if (!response.ok) throw new Error(`EVIDENCE_HTTP_${response.status}`);
    return response.json();
  };

  const waitEvidence = async (predicate, label) => {
    await waitFor(async () => predicate(await sessionEvidence()), label);
  };

  const recordTextTurn = async (input, expected) => {
    const message = element("message", HTMLTextAreaElement);
    const send = element("speak", HTMLButtonElement);
    await waitFor(() => !send.disabled, "text-send-enabled");
    setText(message, input);
    send.click();
    await waitStatus(expected);
  };

  const recordVoiceTurn = async (expectedTranscript, expectedReply) => {
    const voice = element("voice", HTMLButtonElement);
    await waitFor(() => !voice.disabled, "voice-enabled");
    voice.click();
    await waitFor(
      () => (voice.textContent ?? "").includes("Остановить и отправить"),
      "voice-recording-started",
    );
    voice.click();
    try {
      await waitFor(
        () => statusText().includes(expectedTranscript) && statusText().includes(expectedReply),
        "voice-response-complete",
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      throw new Error(`${message};status=${statusText()}`);
    }
  };

  const exportSnapshot = async () => {
    const module = await import("/evidence-export.js");
    const identity = await module.downloadSessionEvidence();
    return `session-${identity.session_sequence}-${identity.participant_role}.json`;
  };

  const postReport = async (payload) => {
    try {
      const response = await fetch(reportUrl, {
        method: "POST",
        headers: { "Content-Type": "text/plain;charset=UTF-8" },
        body: JSON.stringify(payload),
      });
      if (!response.ok) throw new Error(`VOICE_JOURNEY_REPORT_HTTP_${response.status}`);
    } catch {
      // The controller owns the terminal deadline and reports the last published phase.
    }
  };

  const postPhase = async (phase) => {
    await postReport({ kind: "phase", phase });
  };

  const run = async () => {
    try {
      await postPhase("driver-started");
      const root = document.documentElement;
      await waitFor(
        () => root.dataset.vprProviderAutoConnect === "clicked",
        "provider-auto-connect",
      );
      await waitStatus("WebRTC согласован");
      await postPhase("owner-connected");

      const microphone = element("microphone-device", HTMLSelectElement);
      await waitFor(() => microphone.options.length >= 3, "microphone-options");
      microphone.value = "headset-mic";
      microphone.dispatchEvent(new Event("change", { bubbles: true }));
      await postPhase("microphone-selected");

      await recordTextTurn("Текстовый вопрос владельца", "Текстовый ответ владельцу");
      await postPhase("owner-text-complete");
      await recordVoiceTurn("Привет из браузера", "Голосовой ответ владельцу");
      await postPhase("owner-voice-complete");
      await waitEvidence(
        (snapshot) => snapshot.canonical_playback_proven === true
          && snapshot.av_sync_proven === true
          && snapshot.voice_attempts?.some(
            (attempt) => attempt.status === "completed"
              && attempt.canonical_playback_confirmed === true,
          ),
        "owner-canonical-playback",
      );

      await waitFor(
        () => (window.__vprRequestedMicrophones ?? []).includes("headset-mic"),
        "selected-microphone-used",
      );

      const interrupt = element("interrupt", HTMLButtonElement);
      await waitFor(() => !interrupt.disabled, "interrupt-enabled");
      interrupt.click();
      await waitEvidence(
        (snapshot) => snapshot.media_events?.some(
          (event) => event.kind === "interruption_stopped",
        ),
        "interrupt-evidence",
      );

      const interruptPayloads = [...(window.__vprInterruptPayloads ?? [])];
      const setPeerState = window.__vprSetPeerConnectionState;
      if (typeof setPeerState !== "function") throw new Error("VOICE_JOURNEY_PEER_CONTROL_MISSING");
      setPeerState("disconnected");
      await sleep(25);
      setPeerState("connected");
      await waitEvidence(
        (snapshot) => snapshot.media_events?.some(
          (event) => event.kind === "reconnect_restored" && event.elapsed_millis > 0,
        ),
        "reconnect-evidence",
      );

      const ownerEvidence = await sessionEvidence();
      await postPhase("owner-evidence-captured");

      const message = element("message", HTMLTextAreaElement);
      const send = element("speak", HTMLButtonElement);
      await waitFor(() => !send.disabled, "failure-turn-enabled");
      setText(message, "Спровоцируй отказ провайдера");
      send.click();
      await waitStatus("PROVIDER_UNAVAILABLE");

      await recordTextTurn("Восстановление после отказа", "Ответ после восстановления");
      await postPhase("owner-recovery-complete");
      const recoveredEvidence = await sessionEvidence();

      const close = element("close", HTMLButtonElement);
      close.click();
      await waitStatus("Сессия закрыта");
      const ownerExport = await exportSnapshot();
      await postPhase("owner-closed-exported");

      const audience = element("session-audience", HTMLSelectElement);
      audience.value = "visitor";
      audience.dispatchEvent(new Event("change", { bubbles: true }));
      element("consent", HTMLInputElement).checked = true;
      const connect = element("connect", HTMLButtonElement);
      await waitFor(() => !connect.disabled, "visitor-connect-enabled");
      connect.click();
      await waitStatus("Visitor-сессия WebRTC согласована");
      await postPhase("visitor-connected");

      await recordTextTurn("Текстовый вопрос visitor", "Текстовый ответ visitor");
      await recordVoiceTurn("Что думает владелец?", "В visitor scope нет подтверждённых данных владельца");
      await postPhase("visitor-voice-complete");
      await waitEvidence(
        (snapshot) => snapshot.canonical_playback_proven === true
          && snapshot.av_sync_proven === true
          && snapshot.voice_attempts?.some(
            (attempt) => attempt.status === "completed"
              && attempt.canonical_playback_confirmed === true,
          ),
        "visitor-canonical-playback",
      );
      const visitorEvidence = await sessionEvidence();

      const revoke = element("revoke", HTMLButtonElement);
      revoke.click();
      await waitStatus("Доступ отозван");
      close.click();
      await waitStatus("Сессия закрыта");
      const visitorExport = await exportSnapshot();
      await postPhase("visitor-closed-exported");

      await postReport({
        status: "ok",
        requestedMicrophones: [...(window.__vprRequestedMicrophones ?? [])],
        interruptPayloads,
        ownerEvidence,
        recoveredEvidence,
        visitorEvidence,
        ownerExport,
        visitorExport,
      });
    } catch (error) {
      let evidence = null;
      try {
        evidence = await sessionEvidence();
      } catch {
        // Failure reporting must not hide the original journey error.
      }
      const evidenceSummary = evidence ? {
        participant_role: evidence.participant_role ?? null,
        canonical_playback_proven: evidence.canonical_playback_proven ?? null,
        av_sync_proven: evidence.av_sync_proven ?? null,
        voice_attempts: (evidence.voice_attempts ?? []).map((attempt) => ({
          request_sequence: attempt.request_sequence,
          status: attempt.status,
          canonical_playback_confirmed: attempt.canonical_playback_confirmed,
          failure_code: attempt.failure_code ?? null,
        })),
        media_events: (evidence.media_events ?? []).map((event) => ({
          request_sequence: event.request_sequence,
          kind: event.kind,
        })),
        av_sync_samples: (evidence.av_sync_samples ?? []).map((sample) => ({
          request_sequence: sample.request_sequence,
          sample_sequence: sample.sample_sequence,
        })),
      } : null;
      await postReport({
        status: "failed",
        error: `${error instanceof Error ? error.message : String(error)};evidence=${JSON.stringify(evidenceSummary)}`,
      });
    }
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void run(), { once: true });
  } else {
    void run();
  }
})();
