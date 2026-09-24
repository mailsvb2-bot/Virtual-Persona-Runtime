import { downloadSessionEvidence } from "./evidence-export.js";
import { mountOwnerCapture } from "./owner-capture.js";
import { PlaybackAwareCommandScheduler } from "./voice-command-scheduler.js";
const LIVEKIT_CLIENT_URL = "https://cdn.jsdelivr.net/npm/livekit-client@2.22.3/dist/livekit-client.umd.min.js";
let liveKitLoader = null;
const loadLiveKitSdk = async () => {
    const existing = window.LivekitClient;
    if (existing)
        return existing;
    liveKitLoader ??= new Promise((resolve, reject) => {
        const script = document.createElement("script");
        script.src = LIVEKIT_CLIENT_URL;
        script.async = true;
        script.crossOrigin = "anonymous";
        script.onload = () => {
            const sdk = window.LivekitClient;
            if (sdk)
                resolve(sdk);
            else
                reject(new Error("LIVEKIT_CLIENT_UNAVAILABLE"));
        };
        script.onerror = () => reject(new Error("LIVEKIT_CLIENT_LOAD_FAILED"));
        document.head.append(script);
    });
    return liveKitLoader;
};
const byId = (id) => {
    const element = document.getElementById(id);
    if (!element)
        throw new Error(`missing element ${id}`);
    return element;
};
const video = byId("avatar");
const avatarAudio = byId("avatar-audio");
const stage = document.querySelector(".stage");
const personaPanel = byId("persona-panel");
const audienceSelect = byId("session-audience");
const visitorOption = audienceSelect.querySelector('option[value="visitor"]');
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
const metricStt = byId("metric-stt");
const metricLlm = byId("metric-llm");
const metricLlmFirst = byId("metric-llm-first");
const metricServerTotal = byId("metric-server-total");
const metricTextFirst = byId("metric-text-first");
const metricFirstAudio = byId("metric-first-audio");
const metricVideoReady = byId("metric-video-ready");
const metricAvSync = byId("metric-av-sync");
const metricPlayback = byId("metric-playback");
const metricCost = byId("metric-cost");
let csrfToken = "";
let egressEnabled = false;
let backendStatus = { session_state: "none", avatar_open: false, egress_enabled: false, conversation_readiness: "none", session_audience: null, owner_context_state: "missing", persona_version: 1, reviewed_owner_claims: 0 };
let ownerCaptureReviewed = false;
let peer = null;
let liveKitRoom = null;
let liveKitAudioTrack = null;
let liveKitVideoTrack = null;
let realtimeReadiness = { control: false, audio: false, video: false };
let providerDataChannel = null;
let activeClientControl = null;
let providerPlaybackId = null;
let answerSubmitted = false;
let pendingIce = [];
let capabilities = new Set();
let micStream = null;
let audioContext = null;
let micSource = null;
let micWorklet = null;
let micRequestSequence = null;
let micSamplesSent = 0;
let micPendingPcm = new Uint8Array(0);
let micChunkTail = Promise.resolve();
let micUploadFailure = null;
let recording = false;
let recordingTimer = null;
let textRequestInFlight = false;
let voiceRequestInFlight = false;
let evidenceSessionSequence = 0;
let nextTextRequestSequence = 0;
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
const VOICE_UPLOAD_CHUNK_BYTES = 3_200;
const AUTO_STOP_MILLIS = 29_500;
const AV_SYNC_REFERENCE = "web_rtc_estimated_playout_timestamp";
const AV_SYNC_SAMPLE_COUNT = 3;
const AV_SYNC_SAMPLE_INTERVAL_MILLIS = 100;
const AV_SYNC_MAX_ATTEMPTS = 12;
const setStatus = (text, state = "idle") => {
    statusNode.textContent = text;
    statusNode.dataset.state = state;
};
const formatMillis = (value) => value === null || value === undefined ? "—" : `${Math.round(value)} мс`;
const isSessionEvidenceSnapshot = (value) => {
    if (typeof value !== "object" || value === null)
        return false;
    const candidate = value;
    return typeof candidate.schema_version === "string"
        && candidate.schema_version.startsWith("rt0-owner-lab-session-evidence-")
        && Array.isArray(candidate.text_attempts)
        && Array.isArray(candidate.voice_attempts)
        && Array.isArray(candidate.media_events)
        && Array.isArray(candidate.av_sync_samples);
};
const lastCompleted = (items) => [...items].reverse().find((item) => item.status === "completed");
const lastMediaEvent = (events, kind, requestSequence) => [...events].reverse().find((event) => event.kind === kind
    && (requestSequence === undefined || event.request_sequence === requestSequence));
const sumKnownCost = (usages, key) => {
    const values = usages
        .map((usage) => usage?.[key] ?? null)
        .filter((value) => value !== null && Number.isFinite(value));
    return values.length === 0 ? null : values.reduce((sum, value) => sum + value, 0);
};
const resetTelemetry = () => {
    metricStt.textContent = "—";
    metricLlm.textContent = "—";
    metricLlmFirst.textContent = "—";
    metricServerTotal.textContent = "—";
    metricTextFirst.textContent = "—";
    metricFirstAudio.textContent = "—";
    metricVideoReady.textContent = "—";
    metricAvSync.textContent = "—";
    metricPlayback.textContent = "—";
    metricCost.textContent = "нет измеренных данных";
};
const renderTelemetry = (snapshot) => {
    const voice = lastCompleted(snapshot.voice_attempts);
    const text = lastCompleted(snapshot.text_attempts);
    metricStt.textContent = formatMillis(voice?.stt_millis);
    metricLlm.textContent = formatMillis(voice?.llm_millis);
    metricLlmFirst.textContent = formatMillis(voice?.llm_first_meaningful_millis);
    metricServerTotal.textContent = formatMillis(voice?.server_total_millis ?? text?.server_total_millis);
    metricTextFirst.textContent = formatMillis(text?.first_meaningful_response_millis);
    const firstAudio = voice
        ? lastMediaEvent(snapshot.media_events, "audio_started", voice.request_sequence)
        : lastMediaEvent(snapshot.media_events, "audio_started");
    metricFirstAudio.textContent = formatMillis(firstAudio?.elapsed_millis);
    metricVideoReady.textContent = formatMillis(lastMediaEvent(snapshot.media_events, "video_ready")?.elapsed_millis);
    const avSamples = voice
        ? snapshot.av_sync_samples.filter((sample) => sample.request_sequence === voice.request_sequence)
        : snapshot.av_sync_samples;
    if (snapshot.av_sync_proven && avSamples.length > 0) {
        const maxOffset = Math.max(...avSamples.map((sample) => sample.absolute_offset_millis));
        metricAvSync.textContent = `${maxOffset} мс · ${avSamples.length} изм.`;
    }
    else {
        metricAvSync.textContent = voice ? "ещё не доказан" : "—";
    }
    metricPlayback.textContent = snapshot.canonical_playback_proven
        ? "подтверждён"
        : voice ? "ожидание" : "—";
    const usages = [
        ...snapshot.text_attempts.map((attempt) => attempt.llm_usage),
        ...snapshot.voice_attempts.flatMap((attempt) => [attempt.stt_usage, attempt.llm_usage]),
    ];
    const estimated = sumKnownCost(usages, "estimated_cost_microunits");
    const charged = sumKnownCost(usages, "provider_charge_microunits");
    const parts = [];
    if (estimated !== null)
        parts.push(`оценка: ${estimated} μunits`);
    if (charged !== null)
        parts.push(`провайдер: ${charged} μunits`);
    metricCost.textContent = parts.length > 0
        ? parts.join(" · ")
        : "провайдер не сообщил стоимость";
};
const showEvidence = (value) => {
    evidenceNode.textContent = JSON.stringify(value, null, 2);
    if (isSessionEvidenceSnapshot(value))
        renderTelemetry(value);
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
const apiEvidenceJson = async (path, body, requestSequence) => {
    const response = await fetch(path, {
        method: "POST",
        headers: {
            "Content-Type": "application/json",
            "X-VPR-CSRF": csrfToken,
            "X-VPR-Evidence-Request": String(requestSequence),
        },
        body: JSON.stringify(body),
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
const apiBinary = async (path, body, requestSequence) => {
    const response = await fetch(path, {
        method: "POST",
        headers: {
            "Content-Type": "application/octet-stream",
            "X-VPR-CSRF": csrfToken,
            "X-VPR-Evidence-Request": String(requestSequence),
        },
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
const waitForVoiceEvents = async (requestSequence, onSegment) => {
    let finalResult = null;
    while (true) {
        const batch = await api("/api/voice/events", {
            request_sequence: requestSequence,
        });
        for (const event of batch.events) {
            if (event.kind === "segment") {
                onSegment(event.segment);
            }
            else if (event.kind === "complete") {
                finalResult = event.result;
            }
            else {
                throw new Error(event.code);
            }
        }
        if (batch.terminal) {
            if (!finalResult)
                throw new Error("VOICE_STREAM_INCOMPLETE");
            return finalResult;
        }
    }
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
const collectPlayoutTimestamps = (stats, expectedKind) => {
    const timestamps = [];
    stats?.forEach((raw) => {
        const stat = raw;
        if (stat.type !== "inbound-rtp" || !Number.isFinite(stat.estimatedPlayoutTimestamp))
            return;
        const packetsReceived = stat.packetsReceived;
        if (packetsReceived === undefined || !Number.isFinite(packetsReceived) || packetsReceived <= 0)
            return;
        const kind = stat.kind ?? stat.mediaType;
        if (expectedKind && kind !== undefined && kind !== expectedKind)
            return;
        timestamps.push(stat.estimatedPlayoutTimestamp);
    });
    return timestamps;
};
const readAvSyncOffsetMillis = async () => {
    const currentPeer = peer;
    let audio = [];
    let videoOffsets = [];
    if (currentPeer) {
        const stats = await currentPeer.getStats();
        audio = collectPlayoutTimestamps(stats, "audio");
        videoOffsets = collectPlayoutTimestamps(stats, "video");
    }
    else {
        const audioStats = liveKitAudioTrack?.getRTCStatsReport;
        const videoStats = liveKitVideoTrack?.getRTCStatsReport;
        if (!audioStats || !videoStats)
            return null;
        const [audioReport, videoReport] = await Promise.all([
            audioStats.call(liveKitAudioTrack),
            videoStats.call(liveKitVideoTrack),
        ]);
        audio = collectPlayoutTimestamps(audioReport, "audio");
        videoOffsets = collectPlayoutTimestamps(videoReport, "video");
    }
    if (audio.length !== 1 || videoOffsets.length !== 1)
        return null;
    const audioTimestamp = audio[0];
    const videoTimestamp = videoOffsets[0];
    if (audioTimestamp === undefined || videoTimestamp === undefined)
        return null;
    return Math.round(Math.abs(audioTimestamp - videoTimestamp));
};
const collectAvSyncEvidence = async (requestSequence) => {
    let sampleSequence = 1;
    let attempts = 0;
    while (sampleSequence <= AV_SYNC_SAMPLE_COUNT && attempts < AV_SYNC_MAX_ATTEMPTS) {
        attempts += 1;
        const absoluteOffsetMillis = await readAvSyncOffsetMillis();
        if (absoluteOffsetMillis !== null) {
            await api("/api/evidence/av-sync", {
                session_sequence: evidenceSessionSequence,
                request_sequence: requestSequence,
                sample_sequence: sampleSequence,
                reference: AV_SYNC_REFERENCE,
                absolute_offset_millis: absoluteOffsetMillis,
            });
            sampleSequence += 1;
        }
        if (sampleSequence <= AV_SYNC_SAMPLE_COUNT && attempts < AV_SYNC_MAX_ATTEMPTS) {
            await new Promise((resolve) => window.setTimeout(resolve, AV_SYNC_SAMPLE_INTERVAL_MILLIS));
        }
    }
    if (sampleSequence > 1)
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
                voice.audioStartedElapsed = performance.now() - voice.startedAt;
                void postMediaEvidence("audio_started", voice.audioStartedElapsed, voice.requestSequence)
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
const selectedAudience = () => backendStatus.session_audience ?? audienceSelect.value;
const updateAudienceMode = () => {
    personaPanel.hidden = selectedAudience() === "visitor";
};
const updateControls = () => {
    const transportReady = realtimeReadiness.control && backendStatus.session_state === "active";
    const textReady = backendStatus.conversation_readiness !== "none";
    // Microphone input is an independent canonical STT path. It must not wait for the
    // avatar provider's remote output-audio track to be published or recovered.
    const voiceReady = backendStatus.conversation_readiness === "text_and_voice";
    const playbackReady = activeClientControl?.interrupt_requires_playback_id
        ? providerPlaybackId !== null
        : true;
    const clientInterruptReady = transportReady
        && activeClientControl?.interrupt === true
        && playbackReady;
    speakButton.disabled = !transportReady || !textReady || textRequestInFlight || voiceRequestInFlight;
    interruptButton.disabled = !textRequestInFlight && !voiceRequestInFlight && !clientInterruptReady;
    voiceButton.disabled = recording ? false : !transportReady || !voiceReady || textRequestInFlight || voiceRequestInFlight;
    voiceButton.textContent = recording ? "Остановить и отправить" : "Начать говорить";
    revokeButton.disabled = !backendSessionPresent()
        || (backendStatus.session_state === "revoked" && !backendStatus.avatar_open);
    closeButton.disabled = !backendSessionPresent();
    connectButton.disabled = !egressEnabled || backendSessionPresent() || !ownerCaptureReviewed;
    audienceSelect.disabled = backendSessionPresent();
    if (visitorOption)
        visitorOption.disabled = !ownerCaptureReviewed;
    updateAudienceMode();
};
const ownerCapture = mountOwnerCapture({
    api,
    onStateChange: (state) => {
        ownerCaptureReviewed = state.reviewed;
        updateControls();
    },
});
const handleProviderClientEvent = (raw) => {
    if (!raw)
        return;
    void api("/api/avatar/client-event", { message: raw })
        .then((normalized) => {
        if (normalized?.kind === "playback_started") {
            providerPlaybackId = normalized.playback_id;
        }
        else if (normalized?.kind === "playback_done") {
            providerPlaybackId = null;
            voiceCommandScheduler.playbackDone();
        }
        updateControls();
    })
        .catch(() => undefined);
};
const dispatchClientCommand = async (command) => {
    if (command.route.kind === "web_rtc_data_channel") {
        const channel = providerDataChannel;
        if (!channel
            || channel.readyState !== "open"
            || channel.label !== command.route.label) {
            throw new Error("CLIENT_TRANSPORT_UNAVAILABLE");
        }
        channel.send(command.payload);
        return;
    }
    const room = liveKitRoom;
    if (!room)
        throw new Error("CLIENT_TRANSPORT_UNAVAILABLE");
    await room.localParticipant.sendText(command.payload, { topic: command.route.topic });
};
const voiceCommandScheduler = new PlaybackAwareCommandScheduler(dispatchClientCommand);
const attachLiveKitTrack = (track) => {
    if (track.kind === "video") {
        liveKitVideoTrack = track;
        track.attach(video);
        realtimeReadiness.video = true;
        stage?.classList.add("has-video");
        const requestFrame = video.requestVideoFrameCallback;
        if (typeof requestFrame === "function") {
            requestFrame.call(video, () => recordFirstVideoFrame());
        }
        else {
            video.addEventListener("playing", recordFirstVideoFrame, { once: true });
        }
        setStatus("Видео подключено", "ready");
        updateControls();
    }
    else if (track.kind === "audio") {
        liveKitAudioTrack = track;
        realtimeReadiness.audio = true;
        track.attach(avatarAudio);
        if (track.mediaStreamTrack) {
            void attachRemoteAudioEvidence(track.mediaStreamTrack).catch(() => undefined);
        }
        updateControls();
    }
};
const detachLiveKitTrack = (track, element) => {
    try {
        track.detach?.(element);
    }
    catch {
    }
    element.srcObject = null;
};
const handleLiveKitTrackUnsubscribed = (track) => {
    if (track === liveKitAudioTrack) {
        detachLiveKitTrack(track, avatarAudio);
        liveKitAudioTrack = null;
        realtimeReadiness.audio = false;
        setStatus("Аудиопоток аватара потерян. Микрофон и текст остаются доступны; ожидаю восстановление LiveKit…", "error");
        if (voiceRequestInFlight || voiceCommandScheduler.hasActivePlayback) {
            void interruptAvatar();
        }
    }
    if (track === liveKitVideoTrack) {
        detachLiveKitTrack(track, video);
        liveKitVideoTrack = null;
        realtimeReadiness.video = false;
        stage?.classList.remove("has-video");
        setStatus("Видео-поток аватара потерян. Голос остаётся доступен; ожидаю восстановление LiveKit…", "error");
    }
    updateControls();
};
const clearRealtimeMedia = () => {
    realtimeReadiness = { control: false, audio: false, video: false };
    liveKitAudioTrack = null;
    liveKitVideoTrack = null;
    video.srcObject = null;
    avatarAudio.srcObject = null;
    stage?.classList.remove("has-video");
    updateControls();
};
const closePeerTransport = () => {
    voiceCommandScheduler.interrupt();
    stopMicrophoneCapture();
    stopRemoteEvidence();
    providerDataChannel?.close();
    providerDataChannel = null;
    providerPlaybackId = null;
    activeClientControl = null;
    peer?.close();
    peer = null;
    const room = liveKitRoom;
    liveKitRoom = null;
    void room?.disconnect().catch(() => undefined);
    clearRealtimeMedia();
    answerSubmitted = false;
    pendingIce = [];
    capabilities.clear();
};
const handleUnexpectedLiveKitDisconnect = async (room, reason) => {
    if (liveKitRoom !== room)
        return;
    const reasonSuffix = reason === undefined ? "" : ` (reason=${String(reason)})`;
    liveKitRoom = null;
    stopMicrophoneCapture();
    stopRemoteEvidence();
    clearRealtimeMedia();
    setStatus(`LiveKit отключен${reasonSuffix}. Завершаю зависшую сессию…`, "error");
    if (!backendSessionPresent())
        return;
    try {
        await api("/api/session/close", {});
        await syncStatus();
        await refreshSessionEvidence();
        try {
            await downloadSessionEvidence();
            setStatus(`LiveKit отключен${reasonSuffix}. Сессия закрыта, evidence snapshot сохранён. Подключитесь снова.`, "error");
        }
        catch (exportError) {
            setStatus(exportError instanceof Error
                ? `LiveKit отключен${reasonSuffix}. Сессия закрыта; evidence export: ${exportError.message}`
                : `LiveKit отключен${reasonSuffix}. Сессия закрыта; evidence export failed`, "error");
        }
    }
    catch (error) {
        await syncStatus().catch(() => undefined);
        setStatus(error instanceof Error
            ? `LiveKit отключен${reasonSuffix}; cleanup: ${error.message}`
            : `LiveKit отключен${reasonSuffix}; cleanup failed`, "error");
    }
};
const connectWebRtcTransport = async (transport, clientControl) => {
    peer = new RTCPeerConnection({
        iceServers: transport.ice_servers.map((server) => ({
            urls: server.urls,
            ...(server.username ? { username: server.username } : {}),
            ...(server.credential ? { credential: server.credential } : {}),
        })),
    });
    const eventRoute = clientControl?.event_route;
    if (eventRoute?.kind === "web_rtc_data_channel") {
        const channel = peer.createDataChannel(eventRoute.label);
        providerDataChannel = channel;
        channel.onopen = () => updateControls();
        channel.onclose = () => {
            providerPlaybackId = null;
            updateControls();
        };
        channel.onmessage = (event) => {
            const raw = typeof event.data === "string" ? event.data : "";
            handleProviderClientEvent(raw);
        };
    }
    peer.ontrack = (event) => {
        remoteMediaStream ??= new MediaStream();
        if (!remoteMediaStream.getTracks().some((track) => track.id === event.track.id)) {
            remoteMediaStream.addTrack(event.track);
        }
        video.srcObject = remoteMediaStream;
        if (event.track.kind === "video") {
            realtimeReadiness.video = true;
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
            realtimeReadiness.audio = true;
            void attachRemoteAudioEvidence(event.track).catch(() => undefined);
        }
        updateControls();
    };
    peer.onconnectionstatechange = () => {
        if (!peer)
            return;
        const state = peer.connectionState;
        realtimeReadiness.control = state === "connected";
        updateControls();
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
    await peer.setRemoteDescription({ type: transport.offer.kind, sdp: transport.offer.sdp });
    const answer = await peer.createAnswer();
    await peer.setLocalDescription(answer);
    await api("/api/avatar/answer", { kind: answer.type, sdp: answer.sdp ?? "" });
    answerSubmitted = true;
    await flushIce();
};
const connectLiveKitTransport = async (transport) => {
    const sdk = await loadLiveKitSdk();
    const room = new sdk.Room();
    liveKitRoom = room;
    room.on(sdk.RoomEvent.TrackSubscribed, (...args) => {
        const track = args[0];
        if (track)
            attachLiveKitTrack(track);
    });
    room.on(sdk.RoomEvent.TrackUnsubscribed, (...args) => {
        const track = args[0];
        if (track)
            handleLiveKitTrackUnsubscribed(track);
    });
    room.on(sdk.RoomEvent.DataReceived, (...args) => {
        const payload = args[0];
        if (!(payload instanceof Uint8Array))
            return;
        handleProviderClientEvent(new TextDecoder().decode(payload));
    });
    room.on(sdk.RoomEvent.Reconnecting, () => {
        reconnectStartedAt ??= performance.now();
    });
    room.on(sdk.RoomEvent.Reconnected, () => {
        if (reconnectStartedAt !== null) {
            const startedAt = reconnectStartedAt;
            reconnectStartedAt = null;
            void postMediaEvidence("reconnect_restored", performance.now() - startedAt)
                .catch(() => undefined);
        }
    });
    room.on(sdk.RoomEvent.Disconnected, (reason) => {
        void handleUnexpectedLiveKitDisconnect(room, reason);
    });
    await room.connect(transport.server_url, transport.token);
    realtimeReadiness.control = true;
    updateControls();
};
const connectAvatar = async () => {
    if (!consent.checked) {
        setStatus("Нужно явное согласие", "error");
        return;
    }
    connectButton.disabled = true;
    connectEvidenceStartedAt = performance.now();
    resetTelemetry();
    videoEvidencePosted = false;
    reconnectStartedAt = null;
    evidenceSessionSequence = 0;
    nextTextRequestSequence = 0;
    nextVoiceRequestSequence = 0;
    remoteEvidenceAudioContext = new AudioContext();
    void remoteEvidenceAudioContext.resume();
    setStatus("Создаю защищённую сессию…");
    try {
        const audience = audienceSelect.value;
        const start = await api("/api/avatar/start", { consent: true, audience });
        evidenceSessionSequence = start.evidence_session_sequence;
        backendStatus = {
            ...backendStatus,
            session_state: "active",
            avatar_open: true,
            egress_enabled: egressEnabled,
            session_audience: audience,
        };
        capabilities = new Set(start.capabilities);
        activeClientControl = start.client_control;
        if (start.transport.kind === "web_rtc") {
            await connectWebRtcTransport(start.transport, start.client_control);
        }
        else {
            await connectLiveKitTransport(start.transport);
        }
        updateControls();
        const transportName = start.transport.kind === "web_rtc" ? "WebRTC" : "LiveKit";
        setStatus(selectedAudience() === "visitor"
            ? `Visitor-сессия ${transportName} согласована`
            : `${transportName} согласован`, "ready");
        showEvidence({ transport: start.transport.kind, capabilities: [...capabilities] });
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
const resetMicrophoneUpload = () => {
    micRequestSequence = null;
    micSamplesSent = 0;
    micPendingPcm = new Uint8Array(0);
    micChunkTail = Promise.resolve();
    micUploadFailure = null;
};
const cancelMicrophoneInput = async () => {
    const requestSequence = micRequestSequence;
    if (requestSequence !== null) {
        await apiEvidenceJson("/api/voice/input/cancel", {}, requestSequence).catch(() => undefined);
    }
    resetMicrophoneUpload();
};
const stopMicrophoneCapture = () => {
    if (recordingTimer !== null)
        window.clearTimeout(recordingTimer);
    recordingTimer = null;
    micSource?.disconnect();
    micWorklet?.disconnect();
    if (micWorklet)
        micWorklet.port.onmessage = null;
    micStream?.getTracks().forEach((track) => track.stop());
    void audioContext?.close();
    micSource = null;
    micWorklet = null;
    micStream = null;
    audioContext = null;
    recording = false;
    updateControls();
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
const queueMicrophoneChunk = (chunk) => {
    const requestSequence = micRequestSequence;
    if (requestSequence === null || chunk.length === 0 || micUploadFailure)
        return;
    const body = chunk.slice().buffer;
    micChunkTail = micChunkTail.then(async () => {
        if (micUploadFailure)
            return;
        try {
            await apiBinary("/api/voice/input/chunk", body, requestSequence);
        }
        catch (error) {
            micUploadFailure = error instanceof Error ? error : new Error("VOICE_UPLOAD_FAILED");
        }
    });
};
const appendMicrophonePcm = (bytes) => {
    const combined = new Uint8Array(micPendingPcm.length + bytes.length);
    combined.set(micPendingPcm, 0);
    combined.set(bytes, micPendingPcm.length);
    micPendingPcm = combined;
    while (micPendingPcm.length >= VOICE_UPLOAD_CHUNK_BYTES) {
        queueMicrophoneChunk(micPendingPcm.slice(0, VOICE_UPLOAD_CHUNK_BYTES));
        micPendingPcm = micPendingPcm.slice(VOICE_UPLOAD_CHUNK_BYTES);
    }
};
const flushMicrophonePcm = () => {
    if (micPendingPcm.length === 0)
        return;
    queueMicrophoneChunk(micPendingPcm);
    micPendingPcm = new Uint8Array(0);
};
const microphoneCaptureError = (error) => {
    if (!(error instanceof DOMException)) {
        return error instanceof Error ? error : new Error("MIC_CAPTURE_FAILED");
    }
    switch (error.name) {
        case "NotAllowedError":
            return new Error("MIC_PERMISSION_DENIED");
        case "NotFoundError":
            return new Error("MIC_DEVICE_NOT_FOUND");
        case "NotReadableError":
            return new Error("MIC_DEVICE_UNAVAILABLE");
        case "SecurityError":
            return new Error("MIC_SECURITY_DENIED");
        default:
            return new Error(`MIC_CAPTURE_FAILED:${error.name}`);
    }
};
const startMicrophone = async () => {
    if (!navigator.mediaDevices?.getUserMedia)
        throw new Error("MIC_UNAVAILABLE");
    try {
        micStream = await navigator.mediaDevices.getUserMedia({
        audio: {
            channelCount: 1,
            sampleRate: 16_000,
            echoCancellation: true,
            noiseSuppression: true,
            autoGainControl: true,
            },
        });
    }
    catch (error) {
        throw microphoneCaptureError(error);
    }
    audioContext = new AudioContext({ sampleRate: 16_000, latencyHint: "interactive" });
    if (audioContext.sampleRate !== 16_000) {
        stopMicrophoneCapture();
        throw new Error("MIC_SAMPLE_RATE_UNSUPPORTED");
    }
    await audioContext.audioWorklet.addModule("/mic-worklet.js");
    micSource = audioContext.createMediaStreamSource(micStream);
    micWorklet = new AudioWorkletNode(audioContext, "vpr-mic-capture");
    nextVoiceRequestSequence += 1;
    const requestSequence = nextVoiceRequestSequence;
    micRequestSequence = requestSequence;
    micSamplesSent = 0;
    micPendingPcm = new Uint8Array(0);
    micChunkTail = Promise.resolve();
    micUploadFailure = null;
    const started = await apiEvidenceJson("/api/voice/input/start", {}, requestSequence);
    if (started.request_sequence !== requestSequence) {
        throw new Error("VOICE_STREAM_SEQUENCE_MISMATCH");
    }
    micWorklet.port.onmessage = (event) => {
        if (!recording)
            return;
        const samples = new Float32Array(event.data);
        const remaining = MAX_VOICE_SAMPLES - micSamplesSent;
        if (remaining <= 0) {
            void finishMicrophoneTurn();
            return;
        }
        const bounded = samples.length > remaining ? samples.subarray(0, remaining) : samples;
        if (bounded.length === 0)
            return;
        appendMicrophonePcm(new Uint8Array(encodeS16Le(bounded)));
        micSamplesSent += bounded.length;
        if (micSamplesSent >= MAX_VOICE_SAMPLES)
            void finishMicrophoneTurn();
    };
    recording = true;
    micSource.connect(micWorklet);
    micWorklet.connect(audioContext.destination);
    recordingTimer = window.setTimeout(() => void finishMicrophoneTurn(), AUTO_STOP_MILLIS);
    setStatus("Слушаю… PCM передаётся в распознавание во время речи; нажмите ещё раз, чтобы закончить", "ready");
    updateControls();
};
const finishMicrophoneTurn = async () => {
    if (!recording)
        return;
    const requestSequence = micRequestSequence;
    const samplesSent = micSamplesSent;
    stopMicrophoneCapture();
    if (requestSequence === null) {
        resetMicrophoneUpload();
        setStatus("Поток микрофона не был создан", "error");
        return;
    }
    flushMicrophonePcm();
    await micChunkTail;
    if (micUploadFailure) {
        const failure = micUploadFailure;
        await cancelMicrophoneInput();
        setStatus(failure.message, "error");
        return;
    }
    if (samplesSent === 0) {
        await cancelMicrophoneInput();
        setStatus("Микрофон не записал звук", "error");
        return;
    }
    voiceRequestInFlight = true;
    const attemptedRequestSequence = requestSequence;
    let finishAccepted = false;
    updateControls();
    setStatus("Завершаю распознавание и начинаю ответ…");
    activeVoiceEvidence = {
        requestSequence,
        startedAt: performance.now(),
        audioStarted: false,
        audioStartedElapsed: null,
        responseComplete: false,
        speaking: false,
        silentFrames: 0,
    };
    try {
        const started = await apiEvidenceJson("/api/voice/input/finish", {}, requestSequence);
        if (started.request_sequence !== requestSequence)
            throw new Error("VOICE_STREAM_SEQUENCE_MISMATCH");
        finishAccepted = true;
        let deliveryFailure = null;
        const scheduleSegmentDelivery = (segment) => {
            const command = segment.client_command;
            if (!command)
                return;
            void (async () => {
                const sent = await voiceCommandScheduler.dispatch(command);
                if (!sent)
                    return;
                await api("/api/avatar/client-delivery-sent", {
                    evidence_turn_sequence: segment.evidence_turn_sequence,
                    evidence_output_sequence: segment.evidence_output_sequence,
                });
            })().catch((error) => {
                deliveryFailure = error instanceof Error ? error : new Error("CLIENT_TRANSPORT_UNAVAILABLE");
                voiceCommandScheduler.interrupt();
                void api("/api/avatar/interrupt", {}).catch(() => undefined);
                if (activeVoiceEvidence?.requestSequence === requestSequence) {
                    setStatus(deliveryFailure.message, "error");
                }
            });
        };
        const result = await waitForVoiceEvents(requestSequence, scheduleSegmentDelivery);
        if (deliveryFailure)
            throw deliveryFailure;
        const voice = activeVoiceEvidence;
        if (voice?.requestSequence === requestSequence) {
            voice.responseComplete = true;
            if (voice.audioStartedElapsed !== null) {
                await collectAvSyncEvidence(requestSequence).catch(() => undefined);
            }
        }
        await refreshSessionEvidence();
        setStatus(`Вы: ${result.transcript} · Ответ: ${result.reply}`, "ready");
    }
    catch (error) {
        if (!finishAccepted) {
            await apiEvidenceJson("/api/voice/input/cancel", {}, requestSequence).catch(() => undefined);
        }
        if (activeVoiceEvidence?.requestSequence === attemptedRequestSequence)
            activeVoiceEvidence = null;
        await refreshSessionEvidence();
        setStatus(error instanceof Error ? error.message : "Ошибка голосового запроса", "error");
    }
    finally {
        resetMicrophoneUpload();
        voiceRequestInFlight = false;
        updateControls();
    }
};
const toggleVoice = async () => {
    try {
        if (recording) {
            await finishMicrophoneTurn();
        }
        else {
            if (voiceCommandScheduler.hasActivePlayback) {
                await interruptAvatar();
            }
            await startMicrophone();
        }
    }
    catch (error) {
        stopMicrophoneCapture();
        await cancelMicrophoneInput();
        setStatus(error instanceof Error ? error.message : "Ошибка микрофона", "error");
    }
};
const speak = async () => {
    const text = message.value.trim();
    if (!text)
        return;
    const requestSequence = ++nextTextRequestSequence;
    textRequestInFlight = true;
    updateControls();
    try {
        const result = await apiEvidenceJson("/api/text/turn", { text }, requestSequence);
        setStatus(`Ответ: ${result.reply}`, "ready");
        message.value = "";
        await refreshSessionEvidence();
    }
    catch (error) {
        setStatus(error instanceof Error ? error.message : "Ошибка текстового разговора", "error");
    }
    finally {
        textRequestInFlight = false;
        updateControls();
    }
};
const interruptAvatar = async () => {
    const voice = activeVoiceEvidence;
    if (voice?.audioStarted) {
        interruptEvidenceWatch = {
            requestSequence: voice.requestSequence,
            startedAt: performance.now(),
            silentFrames: 0,
        };
    }
    const playbackId = providerPlaybackId;
    const playbackReady = activeClientControl?.interrupt_requires_playback_id
        ? playbackId !== null
        : true;
    const clientReady = !textRequestInFlight
        && realtimeReadiness.control
        && activeClientControl?.interrupt === true
        && playbackReady;
    voiceCommandScheduler.interrupt();
    try {
        if (voiceRequestInFlight) {
            await api("/api/avatar/interrupt", {});
        }
        if (clientReady) {
            const command = await api("/api/avatar/client-interrupt", {
                playback_id: playbackId,
            });
            await dispatchClientCommand(command);
            providerPlaybackId = null;
            updateControls();
            await refreshSessionEvidence();
            return;
        }
        if (!voiceRequestInFlight) {
            await api("/api/avatar/interrupt", {});
        }
        await refreshSessionEvidence();
    }
    catch (error) {
        interruptEvidenceWatch = null;
        setStatus(error instanceof Error ? error.message : "Ошибка прерывания", "error");
        updateControls();
    }
};
const endSession = async (kind) => {
    closePeerTransport();
    try {
        await api(`/api/session/${kind}`, {});
        await syncStatus();
        await refreshSessionEvidence();
        if (kind === "revoke") {
            setStatus("Доступ отозван. Сессию можно закрыть.", "idle");
        }
        else {
            try {
                await downloadSessionEvidence();
                setStatus("Сессия закрыта. Evidence snapshot сохранён.", "idle");
            }
            catch (exportError) {
                setStatus(exportError instanceof Error
                    ? `Сессия закрыта; evidence export: ${exportError.message}`
                    : "Сессия закрыта; evidence export failed", "error");
            }
        }
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
audienceSelect.addEventListener("change", () => {
    if (audienceSelect.value === "visitor" && !ownerCaptureReviewed) {
        audienceSelect.value = "owner";
        setStatus("Visitor-сессия доступна только после подтверждения Persona", "error");
    }
    else {
        setStatus(audienceSelect.value === "visitor"
            ? "Visitor preview: owner-reviewed личный контекст закрыт"
            : "Owner preview: reviewed owner context доступен каноническому runtime", "ready");
    }
    updateControls();
});
connectButton.addEventListener("click", () => void connectAvatar());
speakButton.addEventListener("click", () => void speak());
interruptButton.addEventListener("click", () => void interruptAvatar());
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
    if (backendStatus.session_audience)
        audienceSelect.value = backendStatus.session_audience;
    if (backendStatus.session_audience !== "visitor") {
        await ownerCapture.refresh();
    }
    else {
        personaPanel.hidden = true;
    }
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
