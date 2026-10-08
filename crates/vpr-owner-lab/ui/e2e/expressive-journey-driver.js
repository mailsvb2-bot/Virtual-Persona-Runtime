(() => {
  const reportUrl = "/__journey/report/expressive";
  const sleep = (millis) => new Promise((resolve) => window.setTimeout(resolve, millis));

  const waitFor = async (predicate, label, timeoutMillis = 20_000) => {
    const started = performance.now();
    while (performance.now() - started < timeoutMillis) {
      try {
        if (await predicate()) return;
      } catch {
        // The real UI may still be advancing through an asynchronous provider lifecycle.
      }
      await sleep(20);
    }
    throw new Error(`EXPRESSIVE_JOURNEY_TIMEOUT:${label}`);
  };

  const element = (id, Constructor) => {
    const value = document.getElementById(id);
    if (!(value instanceof Constructor)) {
      throw new Error(`EXPRESSIVE_JOURNEY_CONTROL_MISSING:${id}`);
    }
    return value;
  };

  const statusText = () => element("status", HTMLElement).textContent ?? "";
  const waitStatus = (expected) => waitFor(
    () => statusText().includes(expected),
    `status:${expected}`,
  );

  const commands = () => [...(window.__vprLiveKitCommands ?? [])];

  const fetchEvidence = async () => {
    const response = await fetch("/api/evidence/session", {
      credentials: "same-origin",
      cache: "no-store",
    });
    if (!response.ok) throw new Error(`EXPRESSIVE_EVIDENCE_HTTP_${response.status}`);
    return response.json();
  };


  const waitEvidence = async (predicate, label) => {
    await waitFor(async () => predicate(await fetchEvidence()), `evidence:${label}`);
  };

  const postReport = async (payload) => {
    await fetch(reportUrl, {
      method: "POST",
      headers: { "content-type": "text/plain;charset=UTF-8" },
      body: JSON.stringify(payload),
      cache: "no-store",
    });
  };

  const postPhase = async (phase) => {
    await postReport({ kind: "phase", phase });
  };

  const recordStreamingVoiceTurn = async (transcript, reply) => {
    const voice = element("voice", HTMLButtonElement);
    const playbackDone = window.__vprExpressivePlaybackDone;
    if (typeof playbackDone !== "function") {
      throw new Error("EXPRESSIVE_PLAYBACK_CONTROL_MISSING");
    }
    const initialSpeakCount = commands().filter((command) => command.topic === "did.speak").length;
    const normalize = (value) => String(value).replace(/\s+/g, " ").trim();

    await waitFor(() => !voice.disabled, "voice-enabled");
    voice.click();
    await waitFor(
      () => (voice.textContent ?? "").includes("Остановить и отправить"),
      "voice-recording-started",
    );
    voice.click();

    const spokenParts = [];
    while (normalize(spokenParts.join(" ")) !== normalize(reply)) {
      if (spokenParts.length >= 8) {
        throw new Error("EXPRESSIVE_STREAMING_SEGMENT_LIMIT_EXCEEDED");
      }
      const targetCount = initialSpeakCount + spokenParts.length + 1;
      await waitFor(
        () => commands().filter((command) => command.topic === "did.speak").length >= targetCount,
        `streamed-did-speak-${spokenParts.length + 1}`,
      );
      const speakCommands = commands().filter((command) => command.topic === "did.speak");
      const payload = JSON.parse(speakCommands[targetCount - 1]?.text ?? "{}");
      const input = payload?.script?.input;
      if (typeof input !== "string" || input.trim().length === 0) {
        throw new Error("EXPRESSIVE_STREAMING_SEGMENT_MISSING");
      }
      if (payload?.script?.should_queue_speaks !== true) {
        throw new Error("EXPRESSIVE_SPEAK_QUEUE_FLAG_MISSING");
      }
      spokenParts.push(input);

      await waitFor(
        () => voice.disabled,
        "rt0-evidence-blocks-next-turn-during-playback",
      );
      // Each queued phrase must finish before the scheduler releases the next did.speak.
      // The delayed audio_started route deliberately keeps the provider-done race covered.
      await sleep(50);
      playbackDone();
    }

    if (spokenParts.length < 2) {
      throw new Error("EXPRESSIVE_REPLY_WAS_NOT_STREAMED_BEFORE_LLM_COMPLETION");
    }

    await waitFor(
      () => statusText().includes(`Вы: ${transcript}`) && statusText().includes(`Ответ: ${reply}`),
      "voice-response-complete",
    );
    await waitEvidence(
      (snapshot) => snapshot.media_events?.some(
        (event) => event.request_sequence === 1 && event.kind === "playback_completed",
      ),
      "provider-playback-completed",
    );
    await waitFor(
      () => element("readiness-voice", HTMLElement).textContent === "Готов",
      "voice-readiness-ready",
    );
    await waitFor(
      () => !voice.disabled,
      "rt0-evidence-unblocks-next-turn-after-final-playback",
    );
    return initialSpeakCount + spokenParts.length;
  };

  const run = async () => {
    try {
      await postPhase("driver-started");
      const root = document.documentElement;
      await waitFor(
        () => root.dataset.vprProviderAutoConnect === "clicked",
        "provider-auto-connect",
      );
      await waitStatus("LiveKit согласован");
      await postPhase("connected");

      await waitFor(
        () => element("readiness-text", HTMLElement).textContent === "Готов",
        "text-ready",
      );
      await waitFor(
        () => element("readiness-video", HTMLElement).textContent === "Готов",
        "video-ready",
      );

      const stage = document.querySelector(".stage");
      const avatar = element("avatar", HTMLVideoElement);
      if (!(stage instanceof HTMLElement)) throw new Error("EXPRESSIVE_STAGE_MISSING");
      const stageRect = stage.getBoundingClientRect();
      const avatarRect = avatar.getBoundingClientRect();
      const layout = {
        overflow: document.documentElement.scrollWidth > window.innerWidth,
        objectFit: getComputedStyle(avatar).objectFit,
        stageWidth: stageRect.width,
        stageHeight: stageRect.height,
        avatarWidth: avatarRect.width,
        avatarHeight: avatarRect.height,
      };

      const voice = element("voice", HTMLButtonElement);
      const speak = element("speak", HTMLButtonElement);

      const loseVideo = window.__vprExpressiveLoseVideo;
      const restoreVideo = window.__vprExpressiveRestoreVideo;
      const loseAudio = window.__vprExpressiveLoseAudio;
      const restoreAudio = window.__vprExpressiveRestoreAudio;
      if (
        typeof loseVideo !== "function"
        || typeof restoreVideo !== "function"
        || typeof loseAudio !== "function"
        || typeof restoreAudio !== "function"
      ) {
        throw new Error("EXPRESSIVE_MEDIA_CONTROL_MISSING");
      }

      loseVideo();
      await waitFor(() => !stage.classList.contains("has-video"), "video-lost");
      await waitFor(() => !voice.disabled, "voice-survives-video-loss");
      await waitStatus("Видео-поток аватара потерян. Голос остаётся доступен");

      restoreVideo();
      await waitFor(() => stage.classList.contains("has-video"), "video-restored");
      await waitFor(() => !voice.disabled, "voice-after-video-restore");

      loseAudio();
      await waitFor(() => !voice.disabled && !speak.disabled, "input-survives-audio-loss");
      await waitStatus("Аудиопоток аватара потерян. Микрофон и текст остаются доступны");

      restoreAudio();
      await waitFor(() => !voice.disabled, "voice-after-audio-restore");
      await postPhase("media-recovery-complete");

      const initialSpeakCount = await recordStreamingVoiceTurn(
        "Привет из браузера",
        "Сначала уточню один важный момент, затем продолжу. Третья фраза.",
      );

      await waitEvidence(
        (snapshot) => snapshot.canonical_playback_proven === true
          && snapshot.av_sync_proven === true
          && snapshot.voice_attempts?.some(
            (attempt) => attempt.status === "completed"
              && attempt.canonical_playback_confirmed === true,
          )
          && snapshot.media_events?.some(
            (event) => event.kind === "backend_complete_received",
          )
          && snapshot.media_events?.some(
            (event) => event.kind === "client_delivery_sent",
          ),
        "canonical-playback-and-browser-timing",
      );
      const evidence = await fetchEvidence();
      await postPhase("first-voice-complete");

      const metrics = {
        stt: element("metric-stt", HTMLElement).textContent ?? "",
        llm: element("metric-llm", HTMLElement).textContent ?? "",
        llmFirst: element("metric-llm-first", HTMLElement).textContent ?? "",
        serverTotal: element("metric-server-total", HTMLElement).textContent ?? "",
        backendComplete: element("metric-backend-complete", HTMLElement).textContent ?? "",
        clientDelivery: element("metric-client-delivery", HTMLElement).textContent ?? "",
        providerAudioDelay: element("metric-provider-audio-delay", HTMLElement).textContent ?? "",
        firstAudio: element("metric-first-audio", HTMLElement).textContent ?? "",
        videoReady: element("metric-video-ready", HTMLElement).textContent ?? "",
        avSync: element("metric-av-sync", HTMLElement).textContent ?? "",
        playback: element("metric-playback", HTMLElement).textContent ?? "",
        cost: element("metric-cost", HTMLElement).textContent ?? "",
      };

      const beforeInterrupt = commands();
      if (beforeInterrupt.filter((command) => command.topic === "did.speak").length !== initialSpeakCount) {
        throw new Error("EXPRESSIVE_INITIAL_SPEAK_COUNT_MISMATCH");
      }

      const interrupt = element("interrupt", HTMLButtonElement);
      await waitFor(() => interrupt.disabled, "interrupt-disabled-while-idle");

      voice.click();
      await waitFor(
        () => (voice.textContent ?? "").includes("Остановить и отправить"),
        "second-recording-started",
      );
      voice.click();

      await waitFor(() => !interrupt.disabled, "interrupt-enabled-during-voice-turn");
      if (commands().filter((command) => command.topic === "did.speak").length !== initialSpeakCount) {
        throw new Error("EXPRESSIVE_INTERRUPTED_TURN_SPOKE_TOO_EARLY");
      }
      interrupt.click();
      await waitStatus("TURN_CANCELLED");
      await sleep(800);

      const commandsAfterInterrupt = commands();
      if (commandsAfterInterrupt.filter((command) => command.topic === "did.speak").length !== initialSpeakCount) {
        throw new Error("EXPRESSIVE_INTERRUPTED_TURN_LEAKED_SPEECH");
      }
      if (!commandsAfterInterrupt.some((command) => command.topic === "did.interrupt")) {
        throw new Error("EXPRESSIVE_INTERRUPT_COMMAND_MISSING");
      }
      await postPhase("interrupt-complete");

      const disconnect = window.__vprExpressiveDisconnect;
      if (typeof disconnect !== "function") {
        throw new Error("EXPRESSIVE_DISCONNECT_CONTROL_MISSING");
      }
      disconnect();
      await waitStatus("Сессия закрыта");
      await postPhase("disconnect-complete");

      await postReport({
        status: "ok",
        layout,
        evidence,
        metrics,
        commands: commandsAfterInterrupt,
      });
    } catch (error) {
      await postReport({
        status: "failed",
        error: error instanceof Error ? error.message : String(error),
      }).catch(() => undefined);
    }
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void run(), { once: true });
  } else {
    void run();
  }
})();