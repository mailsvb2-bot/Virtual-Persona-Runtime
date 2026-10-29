"use strict";

(() => {
  const el = (id) => document.getElementById(id);
  const status = el("status");
  const buttons = ["provision", "publish", "pause", "unpublish", "join", "leave"];
  let csrf = "";
  let peer = null;
  let sessionId = null;
  let stopping = false;
  let checking = null;

  function message(text, problem = false) {
    status.textContent = text;
    status.dataset.error = String(problem);
  }

  async function api(path, data) {
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
      body: JSON.stringify(data)
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.code || "Ошибка сервера");
    return payload;
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

  async function ownerRevoke(path) {
    await api(path, { owner_token: ownerToken() });
    message("Публикация отозвана. Удалённая остановка должна подтверждаться отдельно.");
  }

  el("pause").addEventListener("click", () => busy(async () => {
    await ownerRevoke("/api/rt1/owner/pause");
  }));
  el("unpublish").addEventListener("click", () => busy(async () => {
    await ownerRevoke("/api/rt1/owner/unpublish");
  }));

  function cleanupLocal() {
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
    el("video").srcObject = null;
    el("audio").srcObject = null;
  }

  async function leave() {
    if (!sessionId || stopping) return;
    stopping = true;
    const id = sessionId;
    cleanupLocal();
    try {
      const receipt = await api("/api/rt1/visitor/close", { session_id: id });
      if (!receipt.remote_stop_confirmed) {
        throw new Error("Удалённый STOP ещё не подтверждён.");
      }
      sessionId = null;
      message("Звонок завершён, остановка аудио и видео подтверждена.");
    } finally {
      stopping = false;
      el("join").disabled = Boolean(sessionId);
      el("leave").disabled = !sessionId;
    }
  }

  el("leave").addEventListener("click", () => busy(leave));

  el("join").addEventListener("click", () => busy(async () => {
    if (sessionId) return;
    if (!el("consent").checked) throw new Error("Необходимо согласие на сессию.");
    const visitor_token = el("visitor-token").value.trim();
    if (!visitor_token) throw new Error("Введите код приглашения.");
    message("Устанавливается защищённое медиасоединение…");
    const start = await api("/api/rt1/visitor/start", { visitor_token, consent: true });
    sessionId = start.session_id;
    try {
      const transport = start.signal.transport;
      if (!transport || transport.kind !== "web_rtc" || !transport.offer) {
        throw new Error("Worker не предоставил WebRTC offer.");
      }
      const offer = transport.offer;
      peer = new RTCPeerConnection({
        iceServers: Array.isArray(transport.ice_servers)
          ? transport.ice_servers.map((server) => ({
              urls: server.urls,
              username: server.username || undefined,
              credential: server.credential || undefined
            }))
          : []
      });
      const remoteAudio = new MediaStream();
      const remoteVideo = new MediaStream();
      peer.ontrack = (event) => {
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
      peer.onconnectionstatechange = () => {
        if (!peer) return;
        if (peer.connectionState === "connected") message("Звук и видео подключены.");
        if (["failed", "disconnected", "closed"].includes(peer.connectionState)) {
          message("Соединение прервано; завершаем удалённую сессию.", true);
          void leave().catch((error) => message(String(error.message || error), true));
        }
      };
      await peer.setRemoteDescription({ type: "offer", sdp: offer.sdp });
      const answer = await peer.createAnswer();
      await peer.setLocalDescription(answer);
      await api("/api/rt1/visitor/answer", {
        session_id: sessionId,
        sdp: peer.localDescription.sdp
      });
      checking = setInterval(async () => {
        if (!sessionId || stopping) return;
        try {
          const result = await api("/api/rt1/visitor/status", { session_id: sessionId });
          if (!result.active) {
            message("Владелец завершил доступ. Закрываем соединение.");
            await leave();
          }
        } catch (error) {
          message("Не удаётся проверить полномочия сессии: отключаем медиапоток.", true);
          cleanupLocal();
          try { await leave(); } catch (_) { /* Leave remains retryable. */ }
        }
      }, 2000);
      message("Ожидаем аудио и видео от worker…");
    } catch (error) {
      cleanupLocal();
      try { await leave(); } catch (_) { /* Keep session id for STOP retry. */ }
      throw error;
    }
  }));

  window.addEventListener("pagehide", () => {
    if (sessionId) {
      // A best-effort request is not a STOP receipt. The backend ledger
      // remains the source of truth after abrupt tab close.
      const body = JSON.stringify({ session_id: sessionId });
      void fetch("/api/rt1/visitor/close", {
        method: "POST",
        keepalive: true,
        credentials: "same-origin",
        headers: { "Content-Type": "application/json", "X-VPR-CSRF": csrf },
        body
      }).catch(() => {});
      cleanupLocal();
    }
  });
})();
