"use strict";

(() => {
  const el = (id) => document.getElementById(id);
  const status = el("status");
  const buttons = ["provision", "publish", "pause", "unpublish", "join", "leave", "speak", "interrupt", "send-text"];
  let csrf = "";
  let peer = null;
  let liveKitRoom = null;
  let liveKitAudioTrack = null;
  let liveKitVideoTrack = null;
  let sessionId = null;
  let stopping = false;
  let checking = null;
  let microphone = null;
  let voiceEnabled = false;
  let voiceInFlight = false;
  let textInFlight = false;
  let textController = null;
  let textRequestEpoch = 0;
  let microphonePending = false;
  let microphoneRequest = 0;
  let voiceRequestEpoch = 0;
  let pageHidden = false;

  function message(text, problem = false) {
    status.textContent = text;
    status.dataset.error = String(problem);
  }

  async function api(path, data, signal) {
    if (!csrf) {
      const boot = await fetch("/api/bootstrap", { cache: "no-store" });
      if (!boot.ok) throw new Error("Не удалось получить защиту сессии.");
      const payload = await boot.json();
      csrf = payload.csrf_token;
    }
    const response = await fetch(path, {
      method: "POST",
      cache: "no-store",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json", "X-VPR-CSRF": csrf },
      body: JSON.stringify(data),
      signal
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.code || "Ошибка сервера");
    return payload;
  }

  // Best effort only: an HTTP request is not evidence of remote RTP STOP.
  // The server's durable worker remains the authority for cleanup/retries.
  function requestKeepaliveClose(id) {
    const body = JSON.stringify({ session_id: id });
    void fetch("/api/rt1/visitor/close", {
      method: "POST",
      keepalive: true,
      credentials: "same-origin",
      headers: { "Content-Type": "application/json", "X-VPR-CSRF": csrf },
      body
    }).catch(() => {});
  }

  async function busy(task) {
    buttons.forEach((id) => { el(id).disabled = true; });
    try {
      await task();
    } catch (error) {
      message(String(error.message || error), true);
    } finally {
      buttons.forEach((id) => { el(id).disabled = false; });
      el("join").disabled = Boolean(sessionId);
      el("leave").disabled = !sessionId;
      // A concurrent owner action must not re-enable capture while a voice turn is pending.
      el("speak").disabled = !sessionId || stopping || !voiceEnabled || voiceInFlight || microphonePending;
      el("interrupt").disabled = !sessionId || stopping || pageHidden || (!voiceEnabled && !textInFlight) || microphonePending;
      el("send-text").disabled = !sessionId || stopping || pageHidden || textInFlight;
    }
  }

  function ownerToken() {
    const token = el("owner-token").value.trim();
    if (!token) throw new Error("Введите ключ владельца.");
    return token;
  }

  el("provision").addEventListener("click", () => busy(async () => {
    const owner = await api("/api/rt1/owner/provision", {});
    el("owner-token").value = owner.owner_token;
    message("Кабинет создан. Сохраните ключ владельца вне браузера.");
  }));

  el("publish").addEventListener("click", () => busy(async () => {
    const expected = Number(el("persona-version").value);
    if (!Number.isSafeInteger(expected) || expected < 1) {
      throw new Error("Неверная версия Persona.");
    }
    const result = await api("/api/rt1/owner/publish", {
      owner_token: ownerToken(),
      expected_persona_version: expected
    });
    el("invite-token").value = result.visitor_token;
    message("Persona опубликована. Отправьте посетителю код приглашения.");
  }));

  function invalidatePendingText() {
    textRequestEpoch += 1;
    if (textController) textController.abort();
  }

  async function ownerRevoke(path) {
    invalidatePendingText();
    await api(path, { owner_token: ownerToken() });
    message("Публикация отозвана. Удалённая остановка должна подтверждаться отдельно.");
  }

  el("pause").addEventListener("click", () => busy(async () => {
    await ownerRevoke("/api/rt1/owner/pause");
  }));
  el("unpublish").addEventListener("click", () => busy(async () => {
    await ownerRevoke("/api/rt1/owner/unpublish");
  }));

  async function loadLiveKitSdk() {
    if (window.LivekitClient) return window.LivekitClient;
    await new Promise((resolve, reject) => {
      const script = document.createElement("script");
      script.src = "/vendor/livekit-client.umd.js";
      script.async = true;
      script.onload = resolve;
      script.onerror = () => reject(new Error("Не удалось загрузить LiveKit SDK."));
      document.head.append(script);
    });
    if (!window.LivekitClient) throw new Error("LiveKit SDK недоступен.");
    return window.LivekitClient;
  }

  async function connectLiveKit(transport) {
    if (!transport.server_url || !transport.token) {
      throw new Error("Провайдер не предоставил LiveKit-сессию.");
    }
    const sdk = await loadLiveKitSdk();
    const room = new sdk.Room();
    liveKitRoom = room;
    room.on(sdk.RoomEvent.TrackSubscribed, (track) => {
      if (room !== liveKitRoom) return;
      if (track.kind === "video") {
        liveKitVideoTrack = track;
        track.attach(el("video"));
      } else if (track.kind === "audio") {
        liveKitAudioTrack = track;
        track.attach(el("audio"));
        void el("audio").play().catch(() => {
          message("Нажмите «Включить звук», если браузер блокирует воспроизведение.", true);
        });
      }
      if (liveKitAudioTrack && liveKitVideoTrack) {
        message("Видео и звук Persona подключены.");
      }
    });
    room.on(sdk.RoomEvent.TrackUnsubscribed, (track) => {
      if (room !== liveKitRoom) return;
      if (track === liveKitAudioTrack) {
        track.detach(el("audio"));
        liveKitAudioTrack = null;
        el("audio").srcObject = null;
      }
      if (track === liveKitVideoTrack) {
        track.detach(el("video"));
        liveKitVideoTrack = null;
        el("video").srcObject = null;
      }
    });
    room.on(sdk.RoomEvent.Disconnected, () => {
      if (room === liveKitRoom && sessionId && !stopping) {
        void leave().catch((error) => message(String(error.message || error), true));
      }
    });
    // Disconnecting while connect() is pending does not prove the SDK
    // cancelled its underlying transport. A late successful connection
    // must be torn down again after pagehide/revoke/timeout cleanup.
    const connection = room.connect(transport.server_url, transport.token);
    void connection.then(() => {
      if (room !== liveKitRoom) {
        return Promise.resolve(room.disconnect()).catch(() => {});
      }
    }).catch(() => {});
    let timeoutId;
    try {
      await Promise.race([
        connection,
        new Promise((_, reject) => {
          timeoutId = setTimeout(() => reject(new Error("LiveKit не подключился за 15 секунд.")), 15000);
        })
      ]);
    } finally {
      clearTimeout(timeoutId);
    }
    if (room !== liveKitRoom) {
      throw new Error("LiveKit-сессия уже отозвана.");
    }
    if (room.canPlaybackAudio === false) {
      message("LiveKit подключён. Разрешите воспроизведение звука.", true);
    } else {
      message("LiveKit подключён. Ожидаем голос и видео Persona.");
    }
  }

  function disconnectMicrophone() {
    if (!microphone) return null;
    const capture = microphone;
    microphone = null;
    clearTimeout(capture.timeout);
    if (capture.processor) {
      capture.processor.port.onmessage = null;
      capture.processor.disconnect();
    }
    capture.source.disconnect();
    capture.stream.getTracks().forEach((track) => track.stop());
    void capture.context.close().catch(() => {});
    el("speak").textContent = "Говорить";
    return capture;
  }

  async function beginMicrophone() {
    if (!sessionId || stopping || microphone || microphonePending || !voiceEnabled || voiceInFlight) return;
    if (!navigator.mediaDevices?.getUserMedia) {
      throw new Error("Микрофон доступен только в защищённом браузере.");
    }
    const id = sessionId;
    const request = ++microphoneRequest;
    microphonePending = true;
    el("speak").disabled = true;
    let stream = null;
    let context = null;
    let capture = null;
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: true, noiseSuppression: true },
        video: false
      });
      // The permission prompt can outlive an unsuccessful remote STOP.
      // Session identity alone is not sufficient: a failed close retains it.
      if (request !== microphoneRequest || sessionId !== id || stopping || !voiceEnabled || voiceInFlight) return;
      // Reuse the canonical RT0 AudioWorklet rather than UI-thread audio capture.
      context = new (window.AudioContext || window.webkitAudioContext)({
        sampleRate: 16000,
        latencyHint: "interactive"
      });
      if (context.sampleRate !== 16000) {
        throw new Error("Браузер не поддерживает запись с частотой 16 кГц.");
      }
      const source = context.createMediaStreamSource(stream);
      const pcm = [];
      const maxBytes = 16000 * 2 * 30;
      capture = { id, stream, context, source, processor: null, pcm, timeout: null };
      microphone = capture;
      await context.audioWorklet.addModule("/mic-worklet.js");
      // Recheck after another async boundary, including failed STOP and pagehide.
      if (microphone !== capture || request !== microphoneRequest ||
          sessionId !== id || stopping || !voiceEnabled || voiceInFlight) {
        if (microphone === capture) disconnectMicrophone();
        return;
      }
      const processor = new AudioWorkletNode(context, "vpr-mic-capture");
      capture.processor = processor;
      processor.port.onmessage = (event) => {
        if (microphone !== capture || request !== microphoneRequest ||
            sessionId !== id || stopping || !voiceEnabled || voiceInFlight) return;
        const input = new Float32Array(event.data);
        for (let index = 0; index < input.length && pcm.length < maxBytes; index += 1) {
          const value = Math.max(-1, Math.min(1, input[index]));
          const sample = value < 0 ? Math.round(value * 32768) : Math.round(value * 32767);
          pcm.push(sample & 255, (sample >> 8) & 255);
        }
        if (pcm.length >= maxBytes) void endMicrophone();
      };
      source.connect(processor);
      processor.connect(context.destination);
      capture.timeout = setTimeout(() => void endMicrophone(), 30_000);
      el("speak").textContent = "Отправить речь";
      message("Микрофон включён. Говорите, затем нажмите «Отправить речь».");
    } catch (error) {
      if (capture && microphone === capture) disconnectMicrophone();
      throw error;
    } finally {
      // Ownership transfers to disconnectMicrophone only after capture exists.
      // A rejected AudioContext/source construction must not leak the stream.
      if (!capture) {
        if (context) void context.close().catch(() => {});
        if (stream) stream.getTracks().forEach((track) => track.stop());
      }
      // A stale permission request must not unlock a newer session's controls.
      if (request === microphoneRequest) {
        microphonePending = false;
        el("speak").disabled = !sessionId || stopping || !voiceEnabled || voiceInFlight || microphonePending;
      }
    }
  }

  async function endMicrophone() {
    const capture = disconnectMicrophone();
    if (!capture || capture.id !== sessionId || stopping) return;
    if (capture.pcm.length < 2) {
      message("Микрофон не получил аудиоданных.", true);
      return;
    }
    if (voiceInFlight) return;
    voiceInFlight = true;
    const epoch = voiceRequestEpoch;
    el("speak").disabled = true;
    message("Persona обрабатывает голосовой запрос…");
    try {
      const result = await api("/api/rt1/visitor/voice", {
        session_id: capture.id,
        pcm: capture.pcm
      });
      if (sessionId !== capture.id || stopping) return;
      if (epoch !== voiceRequestEpoch) return;
      el("speech-result").textContent =
        "Вы: " + result.transcript + " · Persona: " + result.reply;
      message("Ответ Persona отправлен действующему аватару.");
    } catch (error) {
      // Closing the session wins over any late voice-provider failure.
      // Do not replace a STOP warning or successful close with a stale error.
      if (sessionId === capture.id && !stopping && voiceEnabled) {
        if (epoch === voiceRequestEpoch) {
          message("Голос недоступен для выбранного провайдера или сессии: " +
            String(error.message || error), true);
        }
      }
    } finally {
      voiceInFlight = false;
      el("speak").disabled = !sessionId || stopping || !voiceEnabled;
    }
  }

  el("interrupt").addEventListener("click", () => busy(async () => {
    if (!sessionId || stopping || (!voiceEnabled && !textInFlight)) return;
    invalidatePendingText();
    if (!voiceEnabled) {
      message("Показ ответа отменён. Уже начатая обработка на сервере может завершиться.");
      return;
    }
    const id = sessionId;
    voiceRequestEpoch += 1;
    disconnectMicrophone();
    let stopped;
    try {
      stopped = await api("/api/rt1/visitor/interrupt", { session_id: id });
    } catch (error) {
      // A failed STOP cannot authorize another microphone upload.
      voiceEnabled = false;
      el("speak").disabled = true;
      throw error;
    }
    if (sessionId === id && !stopping) {
      if (stopped.remote_video_stop_confirmed === true) {
        // This STOP closes the whole worker session; never resume its mic.
        voiceEnabled = false;
        el("speak").disabled = true;
        message("Удалённая остановка аудио и видео подтверждена. Завершите сессию.");
      } else {
        message("Ответ прерван. Удалённая остановка видео этим не подтверждается.");
      }
    }
  }));

  el("speak").addEventListener("click", () => {
    if (microphone) void endMicrophone();
    else void beginMicrophone().catch((error) => message(String(error.message || error), true));
  });

  el("send-text").addEventListener("click", async () => {
    if (!sessionId || stopping || pageHidden || textInFlight) return;
    const text = el("visitor-text").value.trim();
    if (!text || text.length > 8192) {
      message("Введите вопрос до 8192 символов.", true);
      return;
    }
    const id = sessionId;
    const epoch = ++textRequestEpoch;
    textInFlight = true;
    const controller = new AbortController();
    textController = controller;
    const timeout = setTimeout(() => controller.abort(), 20000);
    el("send-text").disabled = true;
    el("interrupt").disabled = !sessionId || stopping || microphonePending;
    try {
      const result = await api("/api/rt1/visitor/text", { session_id: id, text }, controller.signal);
      if (sessionId !== id || stopping || pageHidden || epoch !== textRequestEpoch) return;
      el("speech-result").textContent = "Вы: " + text + " · Persona: " + result.reply;
      el("visitor-text").value = "";
      message("Persona ответила на текстовый вопрос.");
    } catch (error) {
      if (sessionId === id && !stopping && !pageHidden && epoch === textRequestEpoch) {
        message(error.name === "AbortError" ? "Текстовый запрос прерван." : String(error.message || error), error.name !== "AbortError");
      }
    } finally {
      clearTimeout(timeout);
      if (textController === controller) {
        textController = null;
        textInFlight = false;
        el("send-text").disabled = !sessionId || stopping || pageHidden;
        el("interrupt").disabled = !sessionId || stopping || pageHidden || (!voiceEnabled && !textInFlight) || microphonePending;
      }
    }
  });

  function cleanupLocal() {
    // Invalidate pending browser permission prompts even if STOP is unconfirmed.
    microphoneRequest += 1;
    voiceRequestEpoch += 1;
    microphonePending = false;
    disconnectMicrophone();
    if (checking) {
      clearInterval(checking);
      checking = null;
    }
    if (peer) {
      peer.ontrack = null;
      peer.onconnectionstatechange = null;
      peer.close();
      peer = null;
    }
    if (liveKitRoom) {
      const room = liveKitRoom;
      liveKitRoom = null;
      try { void Promise.resolve(room.disconnect()).catch(() => {}); } catch (_) { /* Server STOP is separate. */ }
    }
    liveKitAudioTrack = null;
    liveKitVideoTrack = null;
    el("video").srcObject = null;
    el("audio").srcObject = null;
  }

  async function leave() {
    if (!sessionId || stopping) return;
    invalidatePendingText();
    stopping = true;
    // A failed remote STOP must remain retryable, but cannot authorize new speech.
    voiceEnabled = false;
    el("interrupt").disabled = true;
    const id = sessionId;
    el("send-text").disabled = true;
    cleanupLocal();
    try {
      const receipt = await api("/api/rt1/visitor/close", { session_id: id });
      if (!receipt.remote_stop_confirmed) {
        throw new Error("Удалённый STOP ещё не подтверждён.");
      }
      sessionId = null;
      voiceEnabled = false;
      el("speak").disabled = true;
      message("Звонок завершён, остановка аудио и видео подтверждена.");
    } finally {
      stopping = false;
      el("join").disabled = Boolean(sessionId);
      el("leave").disabled = !sessionId;
      el("speak").disabled = !sessionId || stopping || !voiceEnabled || voiceInFlight || microphonePending;
    }
  }

  el("unlock-audio").addEventListener("click", () => busy(async () => {
    if (liveKitRoom && typeof liveKitRoom.startAudio === "function") {
      await liveKitRoom.startAudio();
    }
    await el("audio").play();
    message("Воспроизведение звука разрешено.");
  }));

  el("leave").addEventListener("click", () => busy(leave));

  function waitForIceGathering(connection, timeoutMs = 15000) {
    if (connection.iceGatheringState === "complete") return Promise.resolve();
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        connection.removeEventListener("icegatheringstatechange", check);
        reject(new Error("ICE-соединение не подготовлено. Проверьте сеть."));
      }, timeoutMs);
      function check() {
        if (connection.iceGatheringState === "complete") {
          clearTimeout(timer);
          connection.removeEventListener("icegatheringstatechange", check);
          resolve();
        }
      }
      connection.addEventListener("icegatheringstatechange", check);
      check();
    });
  }

  el("join").addEventListener("click", () => busy(async () => {
    if (sessionId) return;
    if (!el("consent").checked) throw new Error("Необходимо согласие на сессию.");
    const visitor_token = el("visitor-token").value.trim();
    if (!visitor_token) throw new Error("Введите код приглашения.");
    message("Устанавливается защищённое медиасоединение…");
    const start = await api("/api/rt1/visitor/start", { visitor_token, consent: true });
    const joinedId = start.session_id;
    sessionId = joinedId;
    // The tab may have left while CREATE was still in flight and no session
    // ID was available to the pagehide handler. Reconcile that late identity
    // without ever connecting media or reopening a revoked browser session.
    if (pageHidden) {
      voiceEnabled = false;
      stopping = true;
      requestKeepaliveClose(joinedId);
      return;
    }
    // A successful START creates a sender, not yet an accepted browser SDP.
    // Do not expose microphone capture until transport signaling completes.
    const advertisedVoice = start.voice_enabled === true;
    voiceEnabled = false;
    el("send-text").disabled = true;
    el("speak").disabled = true;
    el("interrupt").disabled = true;
    const assertCurrent = () => {
      if (sessionId !== joinedId || stopping) {
        throw new Error("Медиасессия была отозвана во время подключения.");
      }
    };
    try {
      assertCurrent();
      const transport = start.signal.transport;
      if (!transport) throw new Error("Провайдер не предоставил медиасессию.");
      if (transport.kind === "live_kit") {
        await connectLiveKit(transport);
        assertCurrent();
      } else if (transport.kind === "web_rtc" && transport.offer) {
      const offer = transport.offer;
      const connection = new RTCPeerConnection({
        iceServers: Array.isArray(transport.ice_servers)
          ? transport.ice_servers.map((server) => ({
              urls: server.urls,
              username: server.username || undefined,
              credential: server.credential || undefined
            }))
          : []
      });
      peer = connection;
      const gatheredIce = [];
      if (start.diagnostic_rtp === false) {
        connection.onicecandidate = (event) => {
          if (peer !== connection || sessionId !== joinedId || stopping) return;
          if (event.candidate) {
            gatheredIce.push({
              candidate: event.candidate.candidate,
              sdp_mid: event.candidate.sdpMid,
              sdp_mline_index: event.candidate.sdpMLineIndex
            });
          }
        };
      }
      const remoteAudio = new MediaStream();
      const remoteVideo = new MediaStream();
      connection.ontrack = (event) => {
        if (peer !== connection || sessionId !== joinedId || stopping) return;
        const track = event.track;
        if (track.kind === "audio") {
          remoteAudio.addTrack(track);
          el("audio").srcObject = remoteAudio;
          el("audio").play().catch(() => message("Разрешите воспроизведение звука в браузере.", true));
        }
        if (track.kind === "video") {
          remoteVideo.addTrack(track);
          el("video").srcObject = remoteVideo;
          el("video").play().catch(() => message("Нажмите на видео для воспроизведения.", true));
        }
      };
      connection.onconnectionstatechange = () => {
        if (peer !== connection || sessionId !== joinedId || stopping) return;
        if (connection.connectionState === "connected") message("Звук и видео подключены.");
        if (["failed", "disconnected", "closed"].includes(connection.connectionState)) {
          message("Соединение прервано; завершаем удалённую сессию.", true);
          void leave().catch((error) => message(String(error.message || error), true));
        }
      };
      await connection.setRemoteDescription({ type: "offer", sdp: offer.sdp });
      assertCurrent();
      const answer = await connection.createAnswer();
      assertCurrent();
      await connection.setLocalDescription(answer);
      assertCurrent();
      await waitForIceGathering(connection);
      assertCurrent();
      await api("/api/rt1/visitor/answer", {
        session_id: joinedId,
        sdp: connection.localDescription.sdp
      });
      assertCurrent();
      for (const ice of gatheredIce) {
        assertCurrent();
        await api("/api/rt1/visitor/ice", { session_id: joinedId, ...ice });
      }
      } else {
        throw new Error("Неподдерживаемый тип медиасессии.");
      }
      assertCurrent();
      checking = setInterval(async () => {
        if (sessionId !== joinedId || stopping) return;
        try {
          const result = await api("/api/rt1/visitor/status", { session_id: joinedId });
          if (sessionId !== joinedId || stopping) return;
          if (!result.active) {
            message("Владелец завершил доступ. Закрываем соединение.");
            await leave();
          }
        } catch (error) {
          if (sessionId !== joinedId || stopping) return;
          message("Не удаётся проверить полномочия сессии: отключаем медиапоток.", true);
          cleanupLocal();
          try { await leave(); } catch (_) { /* Leave remains retryable. */ }
        }
      }, 2000);
      assertCurrent();
      voiceEnabled = advertisedVoice;
      el("speak").disabled = !voiceEnabled || voiceInFlight || microphonePending;
      el("interrupt").disabled = !voiceEnabled;
      el("send-text").disabled = false;
      if (transport.kind === "web_rtc") message("Ожидаем аудио и видео от Persona…");
    } catch (error) {
      // Late ICE/SDP completion from an old join must never close a newer one.
      if (sessionId === joinedId) {
        cleanupLocal();
        try { await leave(); } catch (_) { /* Keep session id for STOP retry. */ }
      }
      throw error;
    }
  }));

  window.addEventListener("pagehide", () => {
    invalidatePendingText();
    // Invalidate *pending* START as well: its session ID may arrive only
    // after this callback, when no earlier STOP request could name it.
    pageHidden = true;
    if (sessionId) {
      voiceEnabled = false;
      stopping = true;
      requestKeepaliveClose(sessionId);
      cleanupLocal();
    }
  });
  window.addEventListener("pageshow", (event) => {
    // BFCache must never resurrect the old authorization or media sender.
    if (event.persisted && pageHidden) window.location.reload();
  });

  // All handlers above must be installed before the UI is actionable.
  // This is a readiness receipt for the existing single RT1 script, NOT
  // provider/media admission and not permission to reopen a revoked sender.
  for (const id of ["provision", "publish", "pause", "unpublish", "join", "unlock-audio"]) {
    el(id).disabled = false;
  }
  status.dataset.rt1Ready = "true";
})();
