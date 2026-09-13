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
const closePeer = () => {
    peer?.close();
    peer = null;
    video.srcObject = null;
    stage?.classList.remove("has-video");
    answerSubmitted = false;
    pendingIce = [];
    capabilities.clear();
    speakButton.disabled = true;
    interruptButton.disabled = true;
    revokeButton.disabled = true;
    closeButton.disabled = true;
    connectButton.disabled = false;
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
        capabilities = new Set(start.capabilities);
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
        speakButton.disabled = !capabilities.has("text");
        interruptButton.disabled = !capabilities.has("interrupt");
        revokeButton.disabled = false;
        closeButton.disabled = false;
        setStatus("WebRTC согласован", "ready");
        showEvidence({ connectionState: peer.connectionState, capabilities: [...capabilities] });
    }
    catch (error) {
        closePeer();
        setStatus(error instanceof Error ? error.message : "Ошибка подключения", "error");
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
    try {
        await api(`/api/session/${kind}`, {});
        setStatus(kind === "revoke" ? "Доступ отозван" : "Сессия закрыта", "idle");
    }
    catch (error) {
        setStatus(error instanceof Error ? error.message : "Ошибка завершения", "error");
    }
    finally {
        closePeer();
    }
};
connectButton.addEventListener("click", () => void connectAvatar());
speakButton.addEventListener("click", () => void speak());
interruptButton.addEventListener("click", () => void api("/api/avatar/interrupt", {}).catch((error) => setStatus(String(error), "error")));
revokeButton.addEventListener("click", () => void endSession("revoke"));
closeButton.addEventListener("click", () => void endSession("close"));
window.addEventListener("beforeunload", () => peer?.close());
void api("/api/bootstrap")
    .then((bootstrap) => {
    csrfToken = bootstrap.csrf_token;
    if (!bootstrap.egress_enabled) {
        setStatus("Egress выключен на backend", "error");
        connectButton.disabled = true;
    }
    else {
        setStatus("Готов к подключению");
    }
    return api("/api/status");
})
    .then(showEvidence)
    .catch((error) => setStatus(error instanceof Error ? error.message : "Ошибка bootstrap", "error"));
