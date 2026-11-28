"use strict";

(() => {
  const el = (id) => document.getElementById(id);
  const status = el("status");
  const buttons = ["provision", "owner-refresh", "publish", "pause", "unpublish", "copy-invite", "join", "leave", "speak", "interrupt", "send-text"];
  const visitorOnly = window.location.pathname === "/rt1/visit";

  // Fragment credentials are never sent to the server by an HTTP GET.
  // Consume and remove them before bootstrap, media or provider calls.
  function importVisitorInvite() {
    const fragment = window.location.hash;
    if (!fragment) return;
    window.history.replaceState(null, "", window.location.pathname + window.location.search);
    if (!visitorOnly) return;
    const params = new URLSearchParams(fragment.slice(1));
    const token = params.get("invite");
    if (token && /^[a-zA-Z0-9_-]{32,256}$/.test(token)) {
      el("visitor-token").value = token;
    }
  }
  if (visitorOnly) {
    el("owner-panel").hidden = true;
    el("rt1-heading").textContent = "Persona · Посетитель";
    el("rt1-description").textContent =
      "Разговор с Persona доступен по действующему приглашению. Согласие на аудио- и видеосессию требуется отдельно.";
    document.title = "Persona — посетитель RT1";
  }
  importVisitorInvite();
  // Memory is an explicit staging feature. Never display unusable controls
  // or let an absent PostgreSQL integration masquerade as an enabled product.
  el("memory-panel").hidden = true;
  el("memory-admin-refresh").hidden = true;

  function visitorInviteLink(token) {
    if (!/^[a-zA-Z0-9_-]{32,256}$/.test(token)) {
      throw new Error("Сервер вернул некорректный код приглашения.");
    }
    const configured = ownerSnapshot?.public_visitor_origin;
    // The public origin comes ONLY from the authenticated owner/status API.
    // Never accept a URL provided by the invite or visitor's query string.
    const origin = configured || window.location.origin;
    const address = new URL("/rt1/visit", origin);
    if (configured && (address.protocol !== "https:" || address.origin !== configured)) {
      throw new Error("Неверный публичный адрес посетителя.");
    }
    address.hash = "invite=" + encodeURIComponent(token);
    return address.href;
  }

  function clearOwnerInvite() {
    el("invite-token").value = "";
    el("invite-link").value = "";
    el("copy-invite").disabled = true;
  }
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
  // A timed-out HTTP request is not evidence that server-side TTS/PCM stopped.
  // Keep the session closed to new turns until explicit remote STOP/close.
  let textDeliveryUncertain = false;
  let textController = null;
  let textRequestEpoch = 0;
  let microphonePending = false;
  let microphoneRequest = 0;
  let voiceRequestEpoch = 0;
  let pageHidden = false;
  // A status response is only valid for the exact owner key that requested it.
  // This is a read-only projection of the ONE canonical workspace, not a
  // browser-side authority or a second PersonaVersion.
  let ownerSnapshot = null;
  let ownerSnapshotToken = "";
  let ownerStatusEpoch = 0;
  let ownerBusy = false;

  function renderOwnerControls() {
    const key = el("owner-token").value.trim();
    const authorized = key && ownerSnapshotToken === key && ownerSnapshot;
    const publication = authorized ? ownerSnapshot.publication_status : null;
    el("owner-refresh").disabled = visitorOnly || ownerBusy || !key;
    el("provision").disabled = visitorOnly || ownerBusy || Boolean(key);
    el("publish").disabled = visitorOnly || ownerBusy || !authorized || publication === "published";
    el("pause").disabled = visitorOnly || ownerBusy || !authorized || publication !== "published";
    el("unpublish").disabled = visitorOnly || ownerBusy || !authorized ||
      !["published", "paused"].includes(publication);
    el("persona-version").value = authorized ? String(ownerSnapshot.persona_version) : "";
    el("copy-invite").disabled = visitorOnly || ownerBusy || !authorized ||
      publication !== "published" || !el("invite-link").value;
    el("owner-rights-budget").textContent = authorized
      ? "Записей о правах: " + (ownerSnapshot.representation_rights_count ?? "неизвестно") +
        " · Остаток бюджета (учётные единицы): " +
        (ownerSnapshot.budget_remaining_minor_units ?? "не настроен")
      : "";
    const publicationLabel = {
      private: "закрыта", published: "опубликована",
      paused: "приостановлена", unpublished: "снята с публикации"
    };
    el("owner-state").textContent = authorized
      ? "Persona " + ownerSnapshot.persona_id + " · версия " + ownerSnapshot.persona_version +
        " · публикация: " + (publicationLabel[publication] || "не создана")
      : "Введите и проверьте действующий ключ владельца. Версия Persona не задаётся вручную.";
  }

  async function refreshOwnerStatus() {
    const token = el("owner-token").value.trim();
    const epoch = ++ownerStatusEpoch;
    ownerSnapshot = null;
    ownerSnapshotToken = "";
    renderOwnerControls();
    if (!token) return;
    try {
      const snapshot = await api("/api/rt1/owner/status", { owner_token: token });
      if (epoch !== ownerStatusEpoch || el("owner-token").value.trim() !== token) return;
      if (!Number.isSafeInteger(snapshot.persona_version) || snapshot.persona_version < 1 ||
          typeof snapshot.persona_id !== "string" ||
          ![null, "private", "published", "paused", "unpublished"].includes(snapshot.publication_status) ||
          (snapshot.public_visitor_origin != null &&
            (typeof snapshot.public_visitor_origin !== "string" ||
             !/^https:\/\/[a-zA-Z0-9.-]+$/.test(snapshot.public_visitor_origin)))) {
        throw new Error("Сервер вернул некорректный статус Persona.");
      }
      ownerSnapshot = snapshot;
      ownerSnapshotToken = token;
      if (snapshot.publication_status !== "published") clearOwnerInvite();
      renderOwnerControls();
    } catch (error) {
      if (epoch !== ownerStatusEpoch || el("owner-token").value.trim() !== token) return;
      ownerSnapshot = null;
      ownerSnapshotToken = "";
      renderOwnerControls();
      message("Не удалось подтвердить ключ владельца: " + String(error.message || error), true);
    }
  }

  el("owner-token").addEventListener("input", () => {
    // Invalidate immediately, including responses already in flight.
    // A token issued to the previous owner must not remain in the current UI.
    clearOwnerInvite();
    ownerStatusEpoch += 1;
    ownerSnapshot = null;
    ownerSnapshotToken = "";
    renderOwnerControls();
  });
  el("owner-token").addEventListener("change", () => { void refreshOwnerStatus(); });
  el("owner-refresh").addEventListener("click", () => { void refreshOwnerStatus(); });

  function message(text, problem = false) {
    status.textContent = text;
    status.dataset.error = String(problem);
  }

  // Display actionable canonical reason codes, never provider/private details.
  // Keep the exact safe code visible for support and existing browser tests.
  function userError(code) {
    const explanation = {
      AUTH_REVOKED: "Доступ к разговору отозван. Подключитесь заново по действующему приглашению.",
      AUTH_EXPIRED: "Срок доступа истёк. Запросите новое приглашение.",
      AUTH_SCOPE_DENIED: "Для этого действия не хватает разрешений.",
      CONSENT_REQUIRED: "Для разговора необходимо согласие.",
      EGRESS_DENIED: "Отправка запроса этому провайдеру запрещена.",
      PROVIDER_UNAVAILABLE: "Сервис ответа сейчас недоступен. Повторите попытку позже.",
      PROVIDER_RATE_LIMITED: "Достигнут лимит запросов провайдера. Попробуйте позже.",
      PROVIDER_TIMEOUT: "Провайдер не ответил вовремя. Проверьте состояние сеанса.",
      BUDGET_EXHAUSTED: "Достигнут установленный бюджет разговора.",
      TURN_CANCELLED: "Ответ был прерван.",
      PREPARATION_FAILED: "Представление Persona пока не подготовлено.",
      RT1_MICROPHONE_QUEUE_UNAVAILABLE: "Не удалось передать запись микрофона.",
      RT1_REMOTE_MEDIA_STOP_UNCONFIRMED: "Остановка аудио и видео ещё не подтверждена."
    };
    return explanation[code] ? explanation[code] + " (" + code + ")" : code;
  }

  async function api(path, data, signal) {
    if (!csrf) {
      const boot = await fetch(visitorOnly ? "/api/rt1/visitor/bootstrap" : "/api/bootstrap", { cache: "no-store" });
      if (!boot.ok) throw new Error("Не удалось получить защиту сессии.");
      const payload = await boot.json();
      csrf = payload.csrf_token;
      const configured = payload.rt1_memory_configured === true;
      el("memory-panel").hidden = !configured;
      el("memory-admin-refresh").hidden = !configured;
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
    if (!response.ok) throw new Error(userError(payload.code || "Ошибка сервера"));
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
    if (ownerBusy) return;
    ownerBusy = true;
    buttons.forEach((id) => { el(id).disabled = true; });
    if (!visitorOnly) {
      ["registration-begin", "registration-status", "registration-submit",
       "registration-finish", "registration-complete"].forEach((id) => { el(id).disabled = true; });
    }
    try {
      await task();
    } catch (error) {
      message(String(error.message || error), true);
    } finally {
      ownerBusy = false;
      buttons.forEach((id) => { el(id).disabled = false; });
      renderOwnerControls();
      if (!visitorOnly) {
        el("registration-begin").disabled = false;
        el("registration-status").disabled = false;
        renderRegistration(registrationSnapshot);
      }
      el("copy-invite").disabled = visitorOnly || !el("invite-link").value ||
        !ownerSnapshot || ownerSnapshotToken !== el("owner-token").value.trim() ||
        ownerSnapshot.publication_status !== "published";
      el("join").disabled = Boolean(sessionId);
      el("leave").disabled = !sessionId;
      // A concurrent owner action must not re-enable capture while a voice turn is pending.
      el("speak").disabled = !sessionId || stopping || !voiceEnabled || voiceInFlight || microphonePending || textDeliveryUncertain;
      el("interrupt").disabled = !sessionId || stopping || pageHidden || (!voiceEnabled && !textInFlight) || microphonePending;
      el("send-text").disabled = !sessionId || stopping || pageHidden || textInFlight || voiceInFlight || textDeliveryUncertain;
    }
  }

  function ownerToken() {
    const token = el("owner-token").value.trim();
    if (!token) throw new Error("Введите ключ владельца.");
    return token;
  }

  // Owner enrollment shares the existing CSRF and canonical owner controller.
  // No localStorage, second account registry, or public media permissions.
  if (visitorOnly) el("owner-registration").hidden = true;
  let registrationSnapshot = null;

  function enrollmentToken() {
    const token = el("registration-token").value.trim();
    if (!/^[a-fA-F0-9]{64}$/.test(token)) {
      throw new Error("Введите действующий секрет регистрации (64 символа).");
    }
    return token;
  }

  function renderRegistration(snapshot) {
    registrationSnapshot = snapshot;
    const hasToken = /^[a-fA-F0-9]{64}$/.test(el("registration-token").value.trim());
    const current = snapshot && snapshot.current_question;
    const claims = Array.isArray(snapshot?.claims) ? snapshot.claims : [];
    const reviewed = snapshot?.capture_state === "reviewed";
    el("registration-question-label").hidden = !current;
    el("registration-question").textContent = current ? current.prompt : "";
    el("registration-submit").disabled = !current || !hasToken || ownerBusy;
    el("registration-finish").disabled = !snapshot || Boolean(current) || reviewed || ownerBusy;
    const canComplete = !current && claims.length > 0 &&
      claims.every((claim) => claim.owner_reviewed === true) &&
      snapshot.answered_count === snapshot.question_count;
    el("registration-complete").disabled = !canComplete || !hasToken || ownerBusy;
    el("registration-state").textContent = !snapshot
      ? "Начните интервью или введите сохранённый секрет регистрации."
      : "Persona " + snapshot.persona_id + " · ответы " +
        snapshot.answered_count + "/" + snapshot.question_count +
        (reviewed ? " · все утверждения подтверждены" : " · необходимо завершить проверку");
    const container = el("registration-claims");
    container.replaceChildren();
    if (!snapshot || current) return;
    for (const claim of claims) {
      const item = document.createElement("div");
      const statement = document.createElement("p");
      statement.textContent = claim.statement + " (" + claim.kind + ")";
      item.appendChild(statement);
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = claim.owner_reviewed ? "Подтверждено" : "Подтвердить";
      button.disabled = claim.owner_reviewed || ownerBusy;
      button.addEventListener("click", () => busy(async () => {
        const next = await api("/api/rt1/owner/register/approve", {
          enrollment_token: enrollmentToken(), claim_id: claim.claim_id
        });
        renderRegistration(next);
      }));
      item.appendChild(button);
      container.appendChild(item);
    }
  }

  el("registration-token").addEventListener("input", () => renderRegistration(null));
  el("registration-begin").addEventListener("click", () => busy(async () => {
    const id = el("registration-persona-id").value.trim();
    if (!id) throw new Error("Укажите название вашей Persona.");
    const result = await api("/api/rt1/owner/register/begin", { persona_id: id });
    el("registration-token").value = result.enrollment_token;
    el("registration-answer").value = "";
    renderRegistration(result.capture);
    message("Интервью создано. Сохраните секрет регистрации.");
  }));

  el("registration-status").addEventListener("click", () => busy(async () => {
    const capture = await api("/api/rt1/owner/register/status", {
      enrollment_token: enrollmentToken()
    });
    renderRegistration(capture);
  }));

  el("registration-submit").addEventListener("click", () => busy(async () => {
    const answer = el("registration-answer").value.trim();
    if (!answer) throw new Error("Ответ не может быть пустым.");
    const snapshot = await api("/api/rt1/owner/register/answer", {
      enrollment_token: enrollmentToken(), answer
    });
    el("registration-answer").value = "";
    renderRegistration(snapshot);
  }));

  el("registration-finish").addEventListener("click", () => busy(async () => {
    const snapshot = await api("/api/rt1/owner/register/finish", {
      enrollment_token: enrollmentToken()
    });
    renderRegistration(snapshot);
    message("Проверьте и подтвердите каждое утверждение Persona.");
  }));

  el("registration-complete").addEventListener("click", () => busy(async () => {
    const account = await api("/api/rt1/owner/register/complete", {
      enrollment_token: enrollmentToken()
    });
    // Clear the one-time enrollment bearer. No secret is persisted in browser storage.
    el("registration-token").value = "";
    renderRegistration(null);
    el("owner-token").value = account.owner_token;
    await refreshOwnerStatus();
    message("Кабинет и отдельная Persona созданы. Сохраните ключ владельца.");
  }));

  el("provision").addEventListener("click", () => busy(async () => {
    const owner = await api("/api/rt1/owner/provision", {});
    el("owner-token").value = owner.owner_token;
    await refreshOwnerStatus();
    message("Кабинет создан. Сохраните ключ владельца вне браузера.");
  }));

  el("publish").addEventListener("click", () => busy(async () => {
    const token = ownerToken();
    if (!ownerSnapshot || ownerSnapshotToken !== token) {
      throw new Error("Сначала подтвердите ключ владельца и загрузите состояние.");
    }
    const expected = ownerSnapshot.persona_version;
    const epoch = ownerStatusEpoch;
    if (!Number.isSafeInteger(expected) || expected < 1) {
      throw new Error("Неверная версия Persona.");
    }
    const result = await api("/api/rt1/owner/publish", {
      owner_token: token,
      expected_persona_version: expected
    });
    // Owner credentials can change while the server processes publication.
    // A late result belongs to the original owner, not the currently
    // displayed workspace. Never expose its visitor bearer after a switch
    // (including switching away and then back to the same owner token).
    if (epoch !== ownerStatusEpoch || el("owner-token").value.trim() !== token) return;
    const visitorLink = visitorInviteLink(result.visitor_token);
    el("invite-token").value = result.visitor_token;
    el("invite-link").value = visitorLink;
    el("copy-invite").disabled = false;
    await refreshOwnerStatus();
    if (el("owner-token").value.trim() === token && ownerSnapshotToken === token) {
      message("Persona опубликована. Отправьте посетителю код приглашения.");
    }
  }));

  el("copy-invite").addEventListener("click", async () => {
    const link = el("invite-link").value;
    if (!link || visitorOnly || !ownerSnapshot ||
        ownerSnapshotToken !== el("owner-token").value.trim() ||
        ownerSnapshot.publication_status !== "published") return;
    try {
      await navigator.clipboard.writeText(link);
      message(ownerSnapshot?.public_visitor_origin
        ? "Публичная ссылка скопирована. Доступ посетителя зависит от активной публикации."
        : "Локальная ссылка скопирована. На другом устройстве localhost недоступен.");
    } catch (_) {
      el("invite-link").focus();
      el("invite-link").select();
      message("Скопируйте выделенную локальную ссылку вручную.");
    }
  });

  function invalidatePendingText() {
    textRequestEpoch += 1;
    if (textController) textController.abort();
  }

  async function ownerRevoke(path) {
    invalidatePendingText();
    await api(path, { owner_token: ownerToken() });
    clearOwnerInvite();
    await refreshOwnerStatus();
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
    if (!sessionId || stopping || microphone || microphonePending || !voiceEnabled || voiceInFlight || textInFlight || textDeliveryUncertain) return;
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
        el("speak").disabled = !sessionId || stopping || !voiceEnabled || voiceInFlight || microphonePending || textDeliveryUncertain;
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
    el("send-text").disabled = true;
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
      el("speak").disabled = !sessionId || stopping || !voiceEnabled || textInFlight || textDeliveryUncertain;
      el("send-text").disabled = !sessionId || stopping || pageHidden || textInFlight || microphonePending || Boolean(microphone) || textDeliveryUncertain;
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
        textDeliveryUncertain = false;
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
    if (!sessionId || stopping || pageHidden || textInFlight || voiceInFlight || microphonePending || microphone || textDeliveryUncertain) return;
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
    // This request now carries canonical LLM -> TTS/PCM delivery, not just a
    // buffered text completion. Stay below the server's 90s response fence.
    const timeout = setTimeout(() => controller.abort(), 85_000);
    el("send-text").disabled = true;
    el("speak").disabled = true;
    message("Persona формирует и озвучивает ответ…");
    el("interrupt").disabled = !sessionId || stopping || microphonePending;
    try {
      const result = await api("/api/rt1/visitor/text", { session_id: id, text }, controller.signal);
      if (sessionId !== id || stopping || pageHidden || epoch !== textRequestEpoch) return;
      el("speech-result").textContent = "Вы: " + text + " · Persona: " + result.reply;
      el("visitor-text").value = "";
      message("Текстовый ответ передан каноническому голосовому медиатракту Persona.");
    } catch (error) {
      if (sessionId === id && !stopping && !pageHidden && epoch === textRequestEpoch) {
        if (error.name === "AbortError") {
          // Browser cancellation never authorizes the next turn. The remote
          // worker may still be playing PCM; require explicit STOP/close.
          textDeliveryUncertain = true;
          message("Ожидание ответа истекло; удалённая остановка не подтверждена. Нажмите «Прервать» или завершите сессию.", true);
        } else {
          message(String(error.message || error), true);
        }
      }
    } finally {
      clearTimeout(timeout);
      if (textController === controller) {
        textController = null;
        textInFlight = false;
        el("send-text").disabled = !sessionId || stopping || pageHidden || voiceInFlight || microphonePending || Boolean(microphone) || textDeliveryUncertain;
        el("speak").disabled = !sessionId || stopping || pageHidden || !voiceEnabled || voiceInFlight || microphonePending || textDeliveryUncertain;
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
      el("speak").disabled = !sessionId || stopping || !voiceEnabled || voiceInFlight || microphonePending || textDeliveryUncertain;
    }
  }

  el("unlock-audio").addEventListener("click", () => busy(async () => {
    if (liveKitRoom && typeof liveKitRoom.startAudio === "function") {
      await liveKitRoom.startAudio();
    }
    await el("audio").play();
    message("Воспроизведение звука разрешено.");
  }));

  // Browsers may block muted/unmuted video autoplay even after SDP/RTP is
  // connected. The ontrack message already tells the visitor to tap video;
  // make that action real. It never allocates a new session or overrides STOP.
  el("video").addEventListener("click", async () => {
    const video = el("video");
    if (!sessionId || stopping || pageHidden || !video.srcObject) return;
    const id = sessionId;
    try {
      await video.play();
      if (sessionId === id && !stopping && !pageHidden) {
        message("Воспроизведение видео восстановлено.");
      }
    } catch (_) {
      if (sessionId === id && !stopping && !pageHidden) {
        message("Браузер не разрешил воспроизведение видео. Проверьте разрешения и повторите нажатие.", true);
      }
    }
  });

  el("leave").addEventListener("click", () => busy(leave));

  function waitForIceGathering(connection, timeoutMs = 15000) {
    // A failed/closed peer cannot produce a usable browser SDP answer.
    // Do not stall the visitor journey until timeout after a network failure
    // or after STOP has already closed the local PeerConnection.
    return new Promise((resolve, reject) => {
      let finished = false;
      const timer = setTimeout(() => {
        finish(new Error("ICE-соединение не подготовлено. Проверьте сеть."));
      }, timeoutMs);
      function finish(error) {
        if (finished) return;
        finished = true;
        clearTimeout(timer);
        for (const event of ["icegatheringstatechange", "connectionstatechange", "signalingstatechange"]) {
          connection.removeEventListener(event, check);
        }
        if (error) reject(error);
        else resolve();
      }
      function check() {
        if (connection.signalingState === "closed" ||
            ["failed", "closed"].includes(connection.connectionState)) {
          finish(new Error("WebRTC-соединение закрыто или завершилось ошибкой."));
        } else if (connection.iceGatheringState === "complete") {
          finish();
        }
      }
      for (const event of ["icegatheringstatechange", "connectionstatechange", "signalingstatechange"]) {
        connection.addEventListener(event, check);
      }
      check();
    });
  }

  el("join").addEventListener("click", () => busy(async () => {
    if (sessionId) return;
    if (!el("consent").checked) throw new Error("Необходимо согласие на сессию.");
    const visitor_token = el("visitor-token").value.trim();
    if (!visitor_token) throw new Error("Введите код приглашения.");
    message("Устанавливается защищённое медиасоединение…");
    const start = await api("/api/rt1/visitor/start", { visitor_token, consent: true, memory_token: el("memory-token").value.trim() || undefined });
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
      // A fresh verified session, not a timed-out request, resets the fence.
      textDeliveryUncertain = false;
      voiceEnabled = advertisedVoice;
      el("speak").disabled = !voiceEnabled || voiceInFlight || microphonePending || textDeliveryUncertain;
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


  // Memory controls share RT1's canonical CSRF-protected API client and
  // the actual server-owned visitor session. Never use invitation code
  // as a personal identity, or store the memory secret in localStorage.
  const memoryButtonIds = ["memory-enroll", "memory-connect", "memory-enable",
    "memory-disable", "memory-save", "memory-list", "memory-receipt", "memory-forget",
    "memory-erase", "memory-revoke", "memory-rotate"];
  function memoryToken() { return el("memory-token").value.trim(); }
  function requireMemorySession() {
    if (!sessionId || stopping || pageHidden) throw new Error("Сначала подключитесь к аватару.");
    return { session_id: sessionId };
  }
  function memoryPrint(message) { el("memory-output").textContent = message; }
  function memoryAction(id, fn) {
    el(id).addEventListener("click", () => busy(async () => {
      try { await fn(); } catch (error) {
        memoryPrint("Ошибка: " + String(error.message || error));
        throw error;
      }
    }));
  }
  memoryAction("memory-enroll", async () => {
    const response = await api("/api/rt1/visitor/memory/enroll", requireMemorySession());
    el("memory-token").value = response.memory_token;
    memoryPrint("Личный ключ создан. Сохраните его в менеджере паролей. Теперь нажмите «Разрешить память». ");
  });
  memoryAction("memory-connect", async () => {
    const response = await api("/api/rt1/visitor/memory/connect", {
      ...requireMemorySession(), memory_token: memoryToken() });
    memoryPrint(response.memory_connected ? "Память подключена к этой беседе." : "Не удалось подключить память.");
  });
  memoryAction("memory-enable", async () => {
    await api("/api/rt1/visitor/memory/consent", {
      ...requireMemorySession(), memory_token: memoryToken(), enabled: true });
    await api("/api/rt1/visitor/memory/connect", {
      ...requireMemorySession(), memory_token: memoryToken() });
    memoryPrint("Память разрешена и подключена к этой беседе.");
  });
  // Privacy operations can revoke the canonical visitor while STOP is still
  // uncertain. Never drop its session ID on an HTTP error: retain /close retry
  // authority, but fence microphone, text, and all local media immediately.
  async function privacyStopAction(path, data) {
    const identity = requireMemorySession();
    invalidatePendingText();
    voiceEnabled = false;
    textDeliveryUncertain = true;
    cleanupLocal();
    el("send-text").disabled = true;
    el("speak").disabled = true;
    el("interrupt").disabled = true;
    const receipt = await api(path, { ...identity, memory_token: memoryToken(), ...data });
    if (receipt.session_stopped !== true) {
      throw new Error("RT1_MEMORY_REMOTE_STOP_UNCONFIRMED");
    }
    // Only the canonical server STOP receipt permits dropping retry identity.
    if (sessionId === identity.session_id) {
      sessionId = null;
      textDeliveryUncertain = false;
    }
    return receipt;
  }

  memoryAction("memory-disable", async () => {
    await privacyStopAction("/api/rt1/visitor/memory/consent", { enabled: false });
    memoryPrint("Память отключена и заметки удалены. Разговор остановлен.");
  });
  memoryAction("memory-save", async () => {
    const content = el("memory-content").value.trim();
    const key = el("memory-key").value.trim();
    const operation_id = crypto.randomUUID();
    const result = await api("/api/rt1/visitor/memory/remember", {
      ...requireMemorySession(), memory_token: memoryToken(), key, content, operation_id });
    if (result.saved !== true || result.ok !== true) {
      throw new Error("Сохранение не подтверждено сервером.");
    }
    memoryPrint("Сервер подтвердил сохранение заметки: " + key);
  });
  memoryAction("memory-list", async () => {
    const facts = [];
    let cursor = null;
    for (let page = 0; page < 6; page++) {
      const data = await api("/api/rt1/visitor/memory/list", {
        ...requireMemorySession(), memory_token: memoryToken(), cursor });
      facts.push(...data.facts);
      if (!data.next_cursor) { cursor = null; break; }
      if (data.next_cursor === cursor) throw new Error("Повторяющийся курсор памяти.");
      cursor = data.next_cursor;
    }
    if (cursor) throw new Error("Не удалось получить всю память.");
    memoryPrint(facts.length ? facts.map(f => `${f.key}: ${f.content}`).join("\n") : "Память пуста.");
  });
  memoryAction("memory-receipt", async () => {
    const data = await api("/api/rt1/visitor/memory/receipt", {
      ...requireMemorySession(), memory_token: memoryToken() });
    memoryPrint(data.last_explicit_note || "Нет подтверждения запоминания в текущей беседе.");
  });
  memoryAction("memory-forget", async () => {
    const key = el("memory-key").value.trim();
    await api("/api/rt1/visitor/memory/forget", {
      ...requireMemorySession(), memory_token: memoryToken(), key, operation_id: crypto.randomUUID() });
    memoryPrint("Сервер подтвердил удаление заметки: " + key);
  });
  memoryAction("memory-erase", async () => {
    if (!window.confirm("Удалить все воспоминания?")) return;
    await privacyStopAction("/api/rt1/visitor/memory/erase", {});
    memoryPrint("Память удалена и разговор остановлен. Для продолжения подключитесь повторно.");
  });
  memoryAction("memory-revoke", async () => {
    if (!window.confirm("Удалить все воспоминания и навсегда отозвать личный ключ?")) return;
    await privacyStopAction("/api/rt1/visitor/memory/revoke", {});
    el("memory-token").value = "";
    memoryPrint("Ключ отозван, память удалена и разговор остановлен. Создайте новую память для следующего сеанса.");
  });
  memoryAction("memory-rotate", async () => {
    if (!window.confirm("Заменить личный ключ? Старый ключ немедленно перестанет работать.")) return;
    const data = await privacyStopAction("/api/rt1/visitor/memory/rotate", {});
    el("memory-token").value = data.memory_token;
    memoryPrint("Ключ заменён. Сохраните его в менеджере паролей и начните новый разговор; старый разговор остановлен.");
  });

  let memorySummaryDigest = null;
  memoryAction("memory-contradictions", async () => {
    const data = await api("/api/rt1/visitor/memory/contradictions", {
      ...requireMemorySession(), memory_token: memoryToken(), content: el("memory-content").value.trim() });
    memoryPrint(data.candidates.length ? data.candidates.map(f => `${f.key}: ${f.content}`).join("\n") : "Противоречия не найдены. Автоматическая перезапись запрещена.");
  });
  memoryAction("memory-validity", async () => {
    const parseOptional = id => el(id).value.trim() ? Number(el(id).value) : null;
    await api("/api/rt1/visitor/memory/validity", { ...requireMemorySession(),
      memory_token: memoryToken(), key: el("memory-key").value.trim(),
      valid_from: parseOptional("memory-valid-from"), valid_until: parseOptional("memory-valid-until") });
    memoryPrint("Сроки актуальности изменены.");
  });
  memoryAction("memory-summary-draft", async () => {
    const data=await api("/api/rt1/visitor/memory/summary/draft", {
      ...requireMemorySession(),memory_token:memoryToken() });
    memorySummaryDigest=data.source_digest;
    el("memory-summary-content").value=data.text;
    memoryPrint("Проверьте текст и подтвердите его явно. Автоматического одобрения нет.");
  });
  memoryAction("memory-summary-approve", async () => {
    if (!memorySummaryDigest) throw new Error("Сначала подготовьте проект резюме.");
    await api("/api/rt1/visitor/memory/summary/approve", {
      ...requireMemorySession(),memory_token:memoryToken(),
      source_digest:memorySummaryDigest,
      key:el("memory-summary-key").value.trim(),
      content:el("memory-summary-content").value.trim() });
    memoryPrint("Резюме подтверждено и записано.");
  });
  memoryAction("memory-summary-list", async () => {
    const data=await api("/api/rt1/visitor/memory/summary/list",{
      ...requireMemorySession(),memory_token:memoryToken() });
    memoryPrint(data.summaries.length ? data.summaries.map(([key,value])=>`${key}: ${value}`).join("\n") : "Резюме пока нет.");
  });

  // Owner-authorized, aggregate-only memory status; no visitor secrets exposed.
  el("memory-admin-refresh").addEventListener("click", () => busy(async () => {
    const owner_token = el("owner-token").value.trim();
    if (!owner_token) throw new Error("Введите ключ владельца.");
    const data = await api("/api/rt1/owner/memory/admin", { owner_token });
    el("memory-admin-status").textContent =
      `Профилей: ${data.active_subjects}; фактов: ${data.facts}; очередь: ${data.queued_writes}; истекли: ${data.expired_facts}; удаление: ${data.unreconciled_erasures}`;
  }));

  // All handlers above must be installed before the UI is actionable.
  // This is a readiness receipt for the existing single RT1 script, NOT
  // provider/media admission and not permission to reopen a revoked sender.
  for (const id of ["join", "unlock-audio"]) {
    el(id).disabled = false;
  }
  renderOwnerControls();
  status.dataset.rt1Ready = "true";
})();
