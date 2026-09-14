import { mountOwnerCapture } from "./owner-capture.js";
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
const voiceButton = byId("voice");
const statusNode = byId("status");
const evidenceNode = byId("evidence");
let csrfToken = "";
let egressEnabled = false;
let backendStatus = { session_state: "none", avatar_open: false, egress_enabled: false, voice_ready: false, owner_context_state: "missing", persona_version: 1, reviewed_owner_claims: 0 };
let ownerCaptureReviewed = false;
let peer = null;
let answerSubmitted = false;
let pendingIce = [];
let capabilities = new Set();
let micStream = null;
let audioContext = null;
let micSource = null;
let micWorklet = null;
let micChunks = [];
let recording = false;
let recordingTimer = null;
let voiceRequestInFlight = false;
let evidenceSessionSequence = 0;
let nextVoiceRequestSequence = 0;
let connectEvidenceStartedAt = 0;
let videoEvidencePosted = false;
let reconnectStartedAt = null;
let remoteMediaStream = null;
let remoteEvidenceAudioContext = null;
let remoteAudioSource = null;
let remoteAudioAnalyser = null;
let remoteSilentGain = null;
let remoteEvidenceFrame = null;
let baselineRms = 0.002;
let activeVoiceEvidence = null;
let interruptEvidenceWatch = null;
const MAX_VOICE_SAMPLES = 480_000;
const AUTO_STOP_MILLIS = 29_500;
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
const apiBinary = async (path, body, requestSequence) => {
    const response = await fetch(path, {
        method: "POST",
        headers: { "Content-Type": "application/octet-stream", "X-VPR-CSRF": csrfToken, "X-VPR-Evidence-Request": String(requestSequence) },
        body,
        credentials: "same-origin",
        cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) {
        const code = payload.code ?? `HTTP_${response.status}`;
        throw new Error(code);
    }
    return payload;
};
const refreshSessionEvidence = async () => {
    if (evidenceSessionSequence === 0)
        return;
    try {
        showEvidence(await api("/api/evidence/session"));
    }
    catch {
    }
};
const postMediaEvidence = async (kind, elapsedMillis, requestSequence = null) => {
    if (evidenceSessionSequence === 0)
        return;
    await api("/api/evidence/media", {
        session_sequence: evidenceSessionSequence,
        request_sequence: requestSequence,
        kind,
        elapsed_millis: Math.max(0, Math.round(elapsedMillis)),
    });
    await refreshSessionEvidence();
};
const rms = (samples) => {
    let sum = 0;
    for (const sample of samples)
        sum += sample * sample;
    return Math.sqrt(sum / Math.max(1, samples.length));
};
const stopRemoteEvidence = () => {
    if (remoteEvidenceFrame !== null)
        cancelAnimationFrame(remoteEvidenceFrame);
    remoteEvidenceFrame = null;
    remoteAudioSource?.disconnect();
    remoteAudioAnalyser?.disconnect();
    remoteSilentGain?.disconnect();
    void remoteEvidenceAudioContext?.close();
    remoteAudioSource = null;
    remoteAudioAnalyser = null;
    remoteSilentGain = null;
    remoteEvidenceAudioContext = null;
    remoteMediaStream = null;
    activeVoiceEvidence = null;
    interruptEvidenceWatch = null;
    baselineRms = 0.002;
};
const monitorRemoteAudio = () => {
    const analyser = remoteAudioAnalyser;
    if (!analyser)
        return;
    const samples = new Float32Array(analyser.fftSize);
    const tick = () => {
        const current = remoteAudioAnalyser;
        if (!current)
            return;
        current.getFloatTimeDomainData(samples);
        const level = rms(samples);
        if (!activeVoiceEvidence)
            baselineRms = baselineRms * 0.95 + level * 0.05;
        const voice = activeVoiceEvidence;
        const speechThreshold = Math.max(0.015, baselineRms * 3 + 0.003);
        if (voice && level > speechThreshold) {
            voice.speaking = true;
            voice.silentFrames = 0;
            if (!voice.audioStarted) {
                voice.audioStarted = true;
                void postMediaEvidence("audio_started", performance.now() - voice.startedAt, voice.requestSequence)
                    .catch(() => undefined);
            }
        }
        else if (voice?.speaking) {
            voice.silentFrames += 1;
            if (voice.silentFrames >= 6)
                voice.speaking = false;
        }
        if (interruptEvidenceWatch) {
            const silenceThreshold = Math.max(0.008, baselineRms * 1.8 + 0.002);
            interruptEvidenceWatch.silentFrames = level < silenceThreshold
                ? interruptEvidenceWatch.silentFrames + 1
                : 0;
            if (interruptEvidenceWatch.silentFrames >= 4) {
                const watch = interruptEvidenceWatch;
                interruptEvidenceWatch = null;
                void postMediaEvidence("interruption_stopped", performance.now() - watch.startedAt, watch.requestSequence).catch(() => undefined);
            }
        }
        remoteEvidenceFrame = requestAnimationFrame(tick);
    };
    remoteEvidenceFrame = requestAnimationFrame(tick);
};
const attachRemoteAudioEvidence = async (track) => {
    if (!remoteEvidenceAudioContext)
        remoteEvidenceAudioContext = new AudioContext();
    await remoteEvidenceAudioContext.resume();
    remoteAudioSource?.disconnect();
    remoteAudioAnalyser?.disconnect();
    remoteSilentGain?.disconnect();
    remoteAudioSource = remoteEvidenceAudioContext.createMediaStreamSource(new MediaStream([track]));
    remoteAudioAnalyser = remoteEvidenceAudioContext.createAnalyser();
    remoteAudioAnalyser.fftSize = 256;
    remoteSilentGain = remoteEvidenceAudioContext.createGain();
    remoteSilentGain.gain.value = 0;
    remoteAudioSource.connect(remoteAudioAnalyser);
    remoteAudioAnalyser.connect(remoteSilentGain);
    remoteSilentGain.connect(remoteEvidenceAudioContext.destination);
    if (remoteEvidenceFrame === null)
        monitorRemoteAudio();
};
const recordFirstVideoFrame = () => {
    if (videoEvidencePosted || connectEvidenceStartedAt === 0)
        return;
    videoEvidencePosted = true;
    void postMediaEvidence("video_ready", performance.now() - connectEvidenceStartedAt)
        .catch(() => undefined);
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
    interruptButton.disabled = !voiceRequestInFlight && (!transportReady || !capabilities.has("interrupt"));
    voiceButton.disabled = recording ? false : !transportReady || !backendStatus.voice_ready || voiceRequestInFlight;
    voiceButton.textContent = recording ? "Остановить и отправить" : "Начать говорить";
    revokeButton.disabled = !backendSessionPresent()
        || (backendStatus.session_state === "revoked" && !backendStatus.avatar_open);
    closeButton.disabled = !backendSessionPresent();
    connectButton.disabled = !egressEnabled || backendSessionPresent() || !ownerCaptureReviewed;
};
const ownerCapture = mountOwnerCapture({
    api,
    onStateChange: (state) => {
        ownerCaptureReviewed = state.reviewed;
        updateControls();
    },
});
const closePeerTransport = () => {
    stopMicrophoneCapture();
    stopRemoteEvidence();
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
    connectEvidenceStartedAt = performance.now();
    videoEvidencePosted = false;
    reconnectStartedAt = null;
    evidenceSessionSequence = 0;
    nextVoiceRequestSequence = 0;
    remoteEvidenceAudioContext = new AudioContext();
    void remoteEvidenceAudioContext.resume();
    setStatus("Создаю защищённую сессию…");
    try {
        const start = await api("/api/avatar/start", { consent: true });
        evidenceSessionSequence = start.evidence_session_sequence;
        backendStatus = { ...backendStatus, session_state: "active", avatar_open: true, egress_enabled: egressEnabled };
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
            remoteMediaStream ??= new MediaStream();
            if (!remoteMediaStream.getTracks().some((track) => track.id === event.track.id)) {
                remoteMediaStream.addTrack(event.track);
            }
            video.srcObject = remoteMediaStream;
            if (event.track.kind === "video") {
                stage?.classList.add("has-video");
                const requestFrame = video.requestVideoFrameCallback;
                if (typeof requestFrame === "function") {
                    requestFrame.call(video, () => recordFirstVideoFrame());
                }
                else {
                    video.addEventListener("playing", recordFirstVideoFrame, { once: true });
                }
                setStatus("Видео подключено", "ready");
            }
            else if (event.track.kind === "audio") {
                void attachRemoteAudioEvidence(event.track).catch(() => undefined);
            }
        };
        peer.onconnectionstatechange = () => {
            if (!peer)
                return;
            const state = peer.connectionState;
            if ((state === "disconnected" || state === "failed") && reconnectStartedAt === null) {
                reconnectStartedAt = performance.now();
            }
            else if (state === "connected" && reconnectStartedAt !== null) {
                const startedAt = reconnectStartedAt;
                reconnectStartedAt = null;
                void postMediaEvidence("reconnect_restored", performance.now() - startedAt)
                    .catch(() => undefined);
            }
            if (state === "failed")
                setStatus("WebRTC connection failed", "error");
            void refreshSessionEvidence();
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
const stopMicrophoneCapture = () => {
    if (recordingTimer !== null)
        window.clearTimeout(recordingTimer);
    recordingTimer = null;
    micSource?.disconnect();
    micWorklet?.disconnect();
    micStream?.getTracks().forEach((track) => track.stop());
    void audioContext?.close();
    micSource = null;
    micWorklet = null;
    micStream = null;
    audioContext = null;
    recording = false;
    updateControls();
};
const flattenChunks = (chunks) => {
    const total = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
    const output = new Float32Array(total);
    let offset = 0;
    for (const chunk of chunks) {
        output.set(chunk, offset);
        offset += chunk.length;
    }
    return output;
};
const resampleMono = (input, inputRate, outputRate = 16000) => {
    if (inputRate === outputRate)
        return input;
    const outputLength = Math.max(1, Math.floor(input.length * outputRate / inputRate));
    const output = new Float32Array(outputLength);
    const ratio = inputRate / outputRate;
    for (let i = 0; i < outputLength; i += 1) {
        const start = Math.floor(i * ratio);
        const end = Math.min(input.length, Math.max(start + 1, Math.floor((i + 1) * ratio)));
        let sum = 0;
        for (let j = start; j < end; j += 1)
            sum += input[j] ?? 0;
        output[i] = sum / Math.max(1, end - start);
    }
    return output;
};
const encodeS16Le = (input) => {
    const buffer = new ArrayBuffer(input.length * 2);
    const view = new DataView(buffer);
    input.forEach((sample, index) => {
        const clamped = Math.max(-1, Math.min(1, sample));
        const value = clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff;
        view.setInt16(index * 2, Math.round(value), true);
    });
    return buffer;
};
const startMicrophone = async () => {
    micChunks = [];
    micStream = await navigator.mediaDevices.getUserMedia({
        audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true },
    });
    audioContext = new AudioContext();
    await audioContext.audioWorklet.addModule("/mic-worklet.js");
    micSource = audioContext.createMediaStreamSource(micStream);
    micWorklet = new AudioWorkletNode(audioContext, "vpr-mic-capture");
    micWorklet.port.onmessage = (event) => {
        micChunks.push(new Float32Array(event.data));
    };
    micSource.connect(micWorklet);
    micWorklet.connect(audioContext.destination);
    recording = true;
    recordingTimer = window.setTimeout(() => void finishMicrophoneTurn(), AUTO_STOP_MILLIS);
    setStatus("Слушаю… нажмите ещё раз, чтобы отправить", "ready");
    updateControls();
};
const finishMicrophoneTurn = async () => {
    if (!recording || !audioContext)
        return;
    const inputRate = audioContext.sampleRate;
    const samples = flattenChunks(micChunks);
    stopMicrophoneCapture();
    micChunks = [];
    if (samples.length === 0) {
        setStatus("Микрофон не записал звук", "error");
        return;
    }
    voiceRequestInFlight = true;
    let attemptedRequestSequence = null;
    updateControls();
    setStatus("Распознаю и формирую ответ…");
    try {
        const resampled = resampleMono(samples, inputRate);
        const bounded = resampled.length > MAX_VOICE_SAMPLES
            ? resampled.subarray(0, MAX_VOICE_SAMPLES)
            : resampled;
        const pcm = encodeS16Le(bounded);
        nextVoiceRequestSequence += 1;
        const requestSequence = nextVoiceRequestSequence;
        attemptedRequestSequence = requestSequence;
        activeVoiceEvidence = { requestSequence, startedAt: performance.now(), audioStarted: false, speaking: false, silentFrames: 0 };
        const result = await apiBinary("/api/voice/turn", pcm, requestSequence);
        await refreshSessionEvidence();
        setStatus(`Вы: ${result.transcript} · Ответ: ${result.reply}`, "ready");
    }
    catch (error) {
        if (activeVoiceEvidence?.requestSequence === attemptedRequestSequence)
            activeVoiceEvidence = null;
        await refreshSessionEvidence();
        setStatus(error instanceof Error ? error.message : "Ошибка голосового запроса", "error");
    }
    finally {
        voiceRequestInFlight = false;
        updateControls();
    }
};
const toggleVoice = async () => {
    try {
        if (recording)
            await finishMicrophoneTurn();
        else
            await startMicrophone();
    }
    catch (error) {
        stopMicrophoneCapture();
        setStatus(error instanceof Error ? error.message : "Ошибка микрофона", "error");
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
        await refreshSessionEvidence();
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
interruptButton.addEventListener("click", () => {
    const voice = activeVoiceEvidence;
    if (voice?.audioStarted) {
        interruptEvidenceWatch = { requestSequence: voice.requestSequence, startedAt: performance.now(), silentFrames: 0 };
    }
    void api("/api/avatar/interrupt", {})
        .then(() => refreshSessionEvidence())
        .catch((error) => {
        interruptEvidenceWatch = null;
        setStatus(String(error), "error");
    });
});
revokeButton.addEventListener("click", () => void endSession("revoke"));
closeButton.addEventListener("click", () => void endSession("close"));
voiceButton.addEventListener("click", () => void toggleVoice());
window.addEventListener("pagehide", closeBackendOnUnload);
void api("/api/bootstrap")
    .then(async (bootstrap) => {
    csrfToken = bootstrap.csrf_token;
    egressEnabled = bootstrap.egress_enabled;
    await syncStatus();
    ownerCaptureReviewed = backendStatus.owner_context_state === "reviewed";
    await ownerCapture.refresh();
    if (!egressEnabled) {
        setStatus("Egress выключен на backend", "error");
    }
    else if (backendSessionPresent()) {
        setStatus("Найдена незакрытая сессия — доступно безопасное завершение", "error");
    }
    else if (!ownerCaptureReviewed) {
        setStatus("Сначала создайте и подтвердите Persona");
    }
    else {
        setStatus("Persona подтверждена. Готов к подключению", "ready");
    }
    updateControls();
})
    .catch((error) => setStatus(error instanceof Error ? error.message : "Ошибка bootstrap", "error"));
