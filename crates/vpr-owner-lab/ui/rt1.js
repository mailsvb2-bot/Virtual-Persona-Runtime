"use strict";

(() => {
  const el = (id) => document.getElementById(id);
  const status = el("status");
  const buttons = ["provision", "publish", "pause", "unpublish", "join", "leave"];
  let csrf = "";
  let peer = null;
  let liveKitRoom = null;
  let liveKitAudioTrack = null;
  let liveKitVideoTrack = null;
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
    await room.connect(transport.server_url, transport.token);
    if (room.canPlaybackAudio === false) {
      message("LiveKit подключён. Разрешите воспроизведение звука.", true);
    } else {
      message("LiveKit подключён. Ожидаем голос и видео Persona.");
    }
  }

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
    if (liveKitRoom) {
      const room = liveKitRoom;
      liveKitRoom = null;
      try { void room.disconnect(); } catch (_) { /* Server STOP is separate. */ }
    }
    liveKitAudioTrack = null;
    liveKitVideoTrack = null;
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
    sessionId = start.session_id;
    try {
      const transport = start.signal.transport;
      if (!transport) throw new Error("Провайдер не предоставил медиасессию.");
      if (transport.kind === "live_kit") {
        await connectLiveKit(transport);
      } else if (transport.kind === "web_rtc" && transport.offer) {
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
      await waitForIceGathering(peer);
      await api("/api/rt1/visitor/answer", {
        session_id: sessionId,
        sdp: peer.localDescription.sdp
      });
      } else {
        throw new Error("Неподдерживаемый тип медиасессии.");
      }
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
      if (transport.kind === "web_rtc") message("Ожидаем аудио и видео от Persona…");
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
