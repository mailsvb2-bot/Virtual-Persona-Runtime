"use strict";
const byId = (id) => {
    const element = document.getElementById(id);
    if (!element)
        throw new Error(`missing element ${id}`);
    return element;
};
const video = byId("avatar");
const stage = document.querySelector(".stage");
const consent = byId("consent");
const message = byId("message");
const connectButton = byId("connect");
const speakButton = byId("speak");
const interruptButton = byId("interrupt");
const revokeButton = byId("revoke");
const closeButton = byId("close");
const statusNode = byId("status");
const evidenceNode = byId("evidence");
let csrfToken = "";
let egressEnabled = false;
let backendStatus = { session_state: "none", avatar_open: false, egress_enabled: false };
let peer = null;
let answerSubmitted = false;
let pendingIce = [];
let capabilities = new Set();
const setStatus = (text, state = "idle") => {
    statusNode.textContent = text;
    statusNode.dataset.state = state;
};
const showEvidence = (value) => {
    evidenceNode.textContent = JSON.stringify(value, null, 2);
};
const api = async (path, body) => {
    const init = body === undefined
        ? { method: "GET", credentials: "same-origin", cache: "no-store" }
        : {
            method: "POST",
            headers: {
                "Content-Type": "application/json",
                "X-VPR-CSRF": csrfToken,
            },
            body: JSON.stringify(body),
            credentials: "same-origin",
            cache: "no-store",
        };
    const response = await fetch(path, init);
    const payload = await response.json();
    if (!response.ok) {
        const code = payload.code ?? `HTTP_${response.status}`;
        throw new Error(code);
    }
    return payload;
};
const syncStatus = async () => {
    backendStatus = await api("/api/status");
    updateControls();
    showEvidence(backendStatus);
    return backendStatus;
};
const postIce = async (candidate) => {
    await api("/api/avatar/ice", {
        candidate: candidate.candidate ?? null,
        sdp_mid: candidate.sdpMid ?? null,
        sdp_mline_index: candidate.sdpMLineIndex ?? null,
    });
};
const flushIce = async () => {
    const queued = pendingIce;
    pendingIce = [];
    for (const candidate of queued)
        await postIce(candidate);
};
const backendSessionPresent = () => !["none", "closed"].includes(backendStatus.session_state);
const updateControls = () => {
    const transportReady = peer !== null && answerSubmitted && backendStatus.session_state === "active";
    speakButton.disabled = !transportReady || !capabilities.has("text");
    interruptButton.disabled = !transportReady || !capabilities.has("interrupt");
    revokeButton.disabled = !backendSessionPresent()
        || (backendStatus.session_state === "revoked" && !backendStatus.avatar_open);
    closeButton.disabled = !backendSessionPresent();
    connectButton.disabled = !egressEnabled || backendSessionPresent();
};
const closePeerTransport = () => {
    peer?.close();
    peer = null;
    video.srcObject = null;
    stage?.classList.remove("has-video");
    answerSubmitted = false;
    pendingIce = [];
    capabilities.clear();
    updateControls();
};
const connectAvatar = async () => {
    if (!consent.checked) {
        setStatus("Нужно явное согласие", "error");
        return;
    }
    connectButton.disabled = true;
    setStatus("Создаю защищённую сессию…");
    try {
        const start = await api("/api/avatar/start", { consent: true });
        backendStatus = { session_state: "active", avatar_open: true, egress_enabled: egressEnabled };
        capabilities = new Set(start.capabilities);
        updateControls();
        peer = new RTCPeerConnection({
            iceServers: start.ice_servers.map((server) => ({
                urls: server.urls,
                ...(server.username ? { username: server.username } : {}),
                ...(server.credential ? { credential: server.credential } : {}),
            })),
        });
        peer.ontrack = (event) => {
            video.srcObject = event.streams[0] ?? new MediaStream([event.track]);
            stage?.classList.add("has-video");
            setStatus("Видео подключено", "ready");
        };
        peer.onconnectionstatechange = () => {
            if (!peer)
                return;
            showEvidence({ connectionState: peer.connectionState, capabilities: [...capabilities] });
            if (peer.connectionState === "failed")
                setStatus("WebRTC connection failed", "error");
        };
        peer.onicecandidate = (event) => {
            const json = event.candidate?.toJSON();
            const candidate = {
                candidate: json?.candidate ?? null,
                sdpMid: json?.sdpMid ?? null,
                sdpMLineIndex: json?.sdpMLineIndex ?? null,
            };
            if (!answerSubmitted)
                pendingIce.push(candidate);
            else
                void postIce(candidate).catch((error) => setStatus(String(error), "error"));
        };
        await peer.setRemoteDescription({ type: start.offer.kind, sdp: start.offer.sdp });
        const answer = await peer.createAnswer();
        await peer.setLocalDescription(answer);
        await api("/api/avatar/answer", { kind: answer.type, sdp: answer.sdp ?? "" });
        answerSubmitted = true;
        await flushIce();
        updateControls();
        setStatus("WebRTC согласован", "ready");
        showEvidence({ connectionState: peer.connectionState, capabilities: [...capabilities] });
    }
    catch (error) {
        const messageText = error instanceof Error ? error.message : "Ошибка подключения";
        closePeerTransport();
        if (backendSessionPresent()) {
            try {
                await api("/api/session/close", {});
                await syncStatus();
            }
            catch (cleanupError) {
                await syncStatus().catch(() => undefined);
                const cleanupText = cleanupError instanceof Error ? cleanupError.message : "cleanup failed";
                setStatus(`${messageText}; cleanup: ${cleanupText}`, "error");
                updateControls();
                return;
            }
        }
        setStatus(messageText, "error");
        updateControls();
    }
};
const speak = async () => {
    const text = message.value.trim();
    if (!text)
        return;
    speakButton.disabled = true;
    try {
        await api("/api/avatar/speak", { text });
        setStatus("Фраза отправлена", "ready");
    }
    catch (error) {
        setStatus(error instanceof Error ? error.message : "Ошибка отправки", "error");
    }
    finally {
        speakButton.disabled = !capabilities.has("text") || peer === null;
    }
};
const endSession = async (kind) => {
    closePeerTransport();
    try {
        await api(`/api/session/${kind}`, {});
        await syncStatus();
        setStatus(kind === "revoke" ? "Доступ отозван. Сессию можно закрыть." : "Сессия закрыта", "idle");
    }
    catch (error) {
        await syncStatus().catch(() => undefined);
        setStatus(error instanceof Error ? `${error.message}; повторите завершение` : "Ошибка завершения", "error");
    }
    finally {
        updateControls();
    }
};
const closeBackendOnUnload = () => {
    if (!backendSessionPresent() || !csrfToken)
        return;
    void fetch("/api/session/close", {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-VPR-CSRF": csrfToken },
        body: "{}",
        credentials: "same-origin",
        cache: "no-store",
        keepalive: true,
    }).catch(() => undefined);
    closePeerTransport();
};
connectButton.addEventListener("click", () => void connectAvatar());
speakButton.addEventListener("click", () => void speak());
interruptButton.addEventListener("click", () => void api("/api/avatar/interrupt", {}).catch((error) => setStatus(String(error), "error")));
revokeButton.addEventListener("click", () => void endSession("revoke"));
closeButton.addEventListener("click", () => void endSession("close"));
window.addEventListener("pagehide", closeBackendOnUnload);
void api("/api/bootstrap")
    .then(async (bootstrap) => {
    csrfToken = bootstrap.csrf_token;
    egressEnabled = bootstrap.egress_enabled;
    await syncStatus();
    if (!egressEnabled) {
        setStatus("Egress выключен на backend", "error");
    }
    else if (backendSessionPresent()) {
        setStatus("Найдена незакрытая сессия — доступно безопасное завершение", "error");
    }
    else {
        setStatus("Готов к подключению");
    }
    updateControls();
})
    .catch((error) => setStatus(error instanceof Error ? error.message : "Ошибка bootstrap", "error"));
