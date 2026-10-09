import { publishBootstrap } from "./bootstrap-context.js";
import { downloadSessionEvidence } from "./evidence-export.js";
import { authorizedSpeakText, replayTextFrom, resumeSentences, suggestedResumeSentence } from "./interrupted-answer.js";
import { mountOwnerCapture } from "./owner-capture.js";
import { PlaybackAwareCommandScheduler } from "./voice-command-scheduler.js";
import { createRuntimeAudioContext, createRuntimeAudioWorkletNode, createRuntimeMediaStream, createRuntimePeerConnection, mediaRuntime, requestVideoFrame, runtimeFetch, runtimeMediaDevices, setMediaSrcObject, } from "./media-runtime.js";
import { SessionRuntimeState, } from "./session-runtime-state.js";
const LIVEKIT_CLIENT_URL = "/vendor/livekit-client.umd.js";
let liveKitLoader = null;
const loadLiveKitSdk = async () => {
    const injectedSdk = mediaRuntime()?.liveKitSdk;
    if (injectedSdk)
        return injectedSdk;
    const existing = window.LivekitClient;
    if (existing)
        return existing;
    liveKitLoader ??= new Promise((resolve, reject) => {
        const script = document.createElement("script");
        script.src = LIVEKIT_CLIENT_URL;
        script.async = true;
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
const resumeAnswerRow = byId("resume-answer-row");
const resumeAnswerButton = byId("resume-answer");
const resumeAnswerFrom = byId("resume-answer-from");
const revokeButton = byId("revoke");
const closeButton = byId("close");
const voiceButton = byId("voice");
const microphoneSelect = byId("microphone-device");
const microphoneLevel = byId("microphone-level");
const microphoneLevelText = byId("microphone-level-text");
const statusNode = byId("status");
const evidenceNode = byId("evidence");
const metricStt = byId("metric-stt");
const metricLlm = byId("metric-llm");
const metricLlmFirst = byId("metric-llm-first");
const metricServerTotal = byId("metric-server-total");
const metricBackendComplete = byId("metric-backend-complete");
const metricClientDelivery = byId("metric-client-delivery");
const metricProviderAudioDelay = byId("metric-provider-audio-delay");
const metricTextFirst = byId("metric-text-first");
const metricFirstAudio = byId("metric-first-audio");
const metricVideoReady = byId("metric-video-ready");
const metricAvSync = byId("metric-av-sync");
const metricPlayback = byId("metric-playback");
const metricCost = byId("metric-cost");
const readinessText = byId("readiness-text");
const readinessVoice = byId("readiness-voice");
const readinessVideo = byId("readiness-video");
let csrfToken = "";
let egressEnabled = false;
let rt0EvidenceMode = false;
let rt0PlaybackPending = false;
let providerPlaybackInFlight = false;
const sessionState = new SessionRuntimeState();
let ownerCaptureReviewed = false;
let bootstrapComplete = false;
let microphoneDeviceListenerInstalled = false;
let statusSyncTail = Promise.resolve();
let peer = null;
let liveKitRoom = null;
let liveKitAudioTrack = null;
let liveKitVideoTrack = null;
let providerDataChannel = null;
let activeClientControl = null;
let answerSubmitted = false;
let pendingIce = [];
let capabilities = new Set();
let micStream = null;
let audioContext = null;
let micSource = null;
let micWorklet = null;
let micRequestSequence = null;
let micSamplesSent = 0;
let voiceDeliveryGeneration = 0;
let micPendingPcm = new Uint8Array(0);
let micChunkTail = Promise.resolve();
let micUploadFailure = null;
let recording = false;
let recordingTimer = null;
let textRequestInFlight = false;
let voiceRequestInFlight = false;
let evidenceSessionSequence = 0;
const SESSION_EGRESS_FENCE_CHANNEL = "vpr.owner-lab.session-egress-fence.v1";
const sessionEgressFence = typeof BroadcastChannel === "undefined"
    ? null : new BroadcastChannel(SESSION_EGRESS_FENCE_CHANNEL);
let locallyFencedSessionSequence = 0;
let observedBackendSessionSequence = 0;
let nextTextRequestSequence = 0;
let nextVoiceRequestSequence = 0;
let connectEvidenceStartedAt = 0;
let connectJourneyStartedAt = 0;
let videoEvidencePosted = false;
let videoTrackReceivedEvidencePosted = false;
let videoAttachedEvidencePosted = false;
let reconnectStartedAt = null;
let connectionEvidenceTail = Promise.resolve();
let connectionEvidenceFailure = null;
let remoteMediaStream = null;
let remoteEvidenceAudioContext = null;
let remoteAudioSource = null;
let remoteAudioAnalyser = null;
let remoteSilentGain = null;
let remoteEvidenceFrame = null;
let baselineRms = 0.002;
let activeVoiceEvidence = null;
let resumeSourceRequest = null;
const authorizedDeliveredParts = new Map();
let completedAuthorizedReply = null;
let interruptedAnswerSentences = [];
let resumeFromIndex = 0;
let resumedSpeechStartedAt = null;
let interruptEvidenceWatch = null;
let avSyncCollectionPending = false;
let pendingAvSyncEvidence = null;
const MAX_VOICE_SAMPLES = 480_000;
const VOICE_UPLOAD_CHUNK_BYTES = 3_200;
const AUTO_STOP_MILLIS = 29_500;
const MICROPHONE_STORAGE_KEY = "vpr.owner-lab.microphone-device-id";
const AV_SYNC_RTP_REFERENCE = "web_rtc_estimated_playout_timestamp";
const AV_SYNC_MEDIA_ELEMENT_REFERENCE = "html_media_element_current_time";
const AV_SYNC_SAMPLE_COUNT = 3;
const AV_SYNC_SAMPLE_INTERVAL_MILLIS = 100;
const AV_SYNC_MAX_ATTEMPTS = 50;
const UNCONFIRMED_PLAYBACK_SILENCE_RECOVERY_MILLIS = 3_000;
const clearInterruptedAnswer = () => {
    resumeSourceRequest = null;
    authorizedDeliveredParts.clear();
    completedAuthorizedReply = null;
    interruptedAnswerSentences = [];
    resumeFromIndex = 0;
    resumedSpeechStartedAt = null;
    resumeAnswerFrom.replaceChildren();
    resumeAnswerRow.hidden = true;
    resumeAnswerButton.disabled = true;
};
const offerInterruptedAnswer = (sentences, elapsedMillis, offset = 0) => {
    if (sentences.length === 0 || offset >= sentences.length)
        return;
    interruptedAnswerSentences = sentences;
    resumeAnswerFrom.replaceChildren();
    sentences.forEach((sentence, index) => {
        const preview = sentence.length > 110 ? `${sentence.slice(0, 107)}…` : sentence;
        resumeAnswerFrom.add(new Option(`${index + 1}. ${preview}`, String(index)));
    });
    resumeFromIndex = Math.min(sentences.length - 1, offset + suggestedResumeSentence(sentences.slice(offset), elapsedMillis));
    resumeAnswerFrom.value = String(resumeFromIndex);
    resumeAnswerRow.hidden = false;
    resumeAnswerButton.disabled = false;
};
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
        && Array.isArray(candidate.av_sync_samples)
        && Array.isArray(candidate.av_sync_diagnostics);
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
    metricBackendComplete.textContent = "—";
    metricClientDelivery.textContent = "—";
    metricProviderAudioDelay.textContent = "—";
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
    const backendComplete = voice
        ? lastMediaEvent(snapshot.media_events, "backend_complete_received", voice.request_sequence)
        : undefined;
    const clientDelivery = voice
        ? lastMediaEvent(snapshot.media_events, "client_delivery_sent", voice.request_sequence)
        : undefined;
    metricBackendComplete.textContent = formatMillis(backendComplete?.elapsed_millis);
    metricClientDelivery.textContent = formatMillis(clientDelivery?.elapsed_millis);
    metricProviderAudioDelay.textContent =
        firstAudio && clientDelivery
            ? formatMillis(Math.max(0, firstAudio.elapsed_millis - clientDelivery.elapsed_millis))
            : "—";
    metricFirstAudio.textContent = formatMillis(firstAudio?.elapsed_millis);
    metricVideoReady.textContent = formatMillis(lastMediaEvent(snapshot.media_events, "video_ready")?.elapsed_millis);
    const avSamples = voice
        ? snapshot.av_sync_samples.filter((sample) => sample.request_sequence === voice.request_sequence)
        : snapshot.av_sync_samples;
    const avDiagnostic = voice
        ? snapshot.av_sync_diagnostics.find((item) => item.request_sequence === voice.request_sequence)
        : undefined;
    if (snapshot.av_sync_proven && avSamples.length > 0) {
        const maxOffset = Math.max(...avSamples.map((sample) => sample.absolute_offset_millis));
        metricAvSync.textContent = `${maxOffset} мс · ${avSamples.length} изм.`;
    }
    else if (avDiagnostic) {
        const audioIssue = avDiagnostic.audio_issue ?? "ok";
        const videoIssue = avDiagnostic.video_issue ?? "ok";
        metricAvSync.textContent =
            `не доказан · audio=${audioIssue}, video=${videoIssue}, ${avDiagnostic.attempts} попыток`;
    }
    else {
        metricAvSync.textContent = voice ? "ещё не доказан" : "—";
    }
    metricPlayback.textContent = snapshot.canonical_playback_proven
        ? "подтверждён"
        : voice && snapshot.media_events.some((event) => event.request_sequence === voice.request_sequence && event.kind === "playback_recovery_triggered") ? "не подтверждён" : voice ? "ожидание" : "—";
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
    const response = await runtimeFetch(path, init);
    const payload = await response.json();
    if (!response.ok) {
        const code = payload.code ?? `HTTP_${response.status}`;
        throw new Error(code);
    }
    return payload;
};
const apiEvidenceJson = async (path, body, requestSequence) => {
    const response = await runtimeFetch(path, {
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
    const response = await runtimeFetch(path, {
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
const recordMediaEvidence = async (kind, elapsedMillis, requestSequence = null) => {
    if (evidenceSessionSequence === 0)
        return;
    await api("/api/evidence/media", {
        session_sequence: evidenceSessionSequence,
        request_sequence: requestSequence,
        kind,
        elapsed_millis: Math.max(0, Math.round(elapsedMillis)),
    });
};
const postMediaEvidence = async (kind, elapsedMillis, requestSequence = null) => {
    await recordMediaEvidence(kind, elapsedMillis, requestSequence);
    await refreshSessionEvidence();
};
const queueConnectionMediaEvidence = (kind, elapsedMillis) => {
    connectionEvidenceTail = connectionEvidenceTail.then(async () => {
        try {
            await recordMediaEvidence(kind, elapsedMillis);
        }
        catch (error) {
            connectionEvidenceFailure ??= error instanceof Error
                ? error
                : new Error("CONNECTION_EVIDENCE_WRITE_FAILED");
        }
    });
};
const flushConnectionMediaEvidence = async () => {
    while (true) {
        const observedTail = connectionEvidenceTail;
        await observedTail;
        if (observedTail === connectionEvidenceTail)
            break;
    }
    if (connectionEvidenceFailure)
        throw connectionEvidenceFailure;
};
const tryFlushConnectionMediaEvidence = async () => {
    try {
        await flushConnectionMediaEvidence();
        return null;
    }
    catch (error) {
        return error instanceof Error ? error : new Error("CONNECTION_EVIDENCE_FLUSH_FAILED");
    }
};
const selectPlayoutTimestamp = (stats, expectedKind, previousPackets) => {
    const packetCounts = new Map();
    if (!stats) {
        return {
            timestamp: null,
            packetCounts,
            issue: "stats_unavailable",
            diagnostic: `${expectedKind}: stats unavailable`,
        };
    }
    let inboundForKind = 0;
    let excludedRtx = 0;
    let missingTimestamp = 0;
    let missingSenderReportMapping = 0;
    const candidates = [];
    stats.forEach((raw, key) => {
        const stat = raw;
        if (stat.type !== "inbound-rtp")
            return;
        const kind = stat.kind ?? stat.mediaType;
        if (kind !== undefined && kind !== expectedKind)
            return;
        inboundForKind += 1;
        const packetsReceived = stat.packetsReceived;
        if (packetsReceived === undefined || !Number.isFinite(packetsReceived) || packetsReceived <= 0) {
            return;
        }
        if (stat.codecId) {
            const codec = stats.get(stat.codecId);
            if (codec?.mimeType?.toLowerCase().endsWith("/rtx")) {
                excludedRtx += 1;
                return;
            }
        }
        const id = String(key);
        packetCounts.set(id, packetsReceived);
        if (!Number.isFinite(stat.estimatedPlayoutTimestamp)) {
            missingTimestamp += 1;
            const remote = stat.remoteId
                ? stats.get(stat.remoteId)
                : undefined;
            if (!remote
                || remote.type !== "remote-outbound-rtp"
                || !Number.isFinite(remote.remoteTimestamp)) {
                missingSenderReportMapping += 1;
            }
            return;
        }
        candidates.push({
            id,
            timestamp: stat.estimatedPlayoutTimestamp,
            packetsReceived,
        });
    });
    if (candidates.length === 1) {
        const only = candidates[0];
        const previous = previousPackets?.get(only.id);
        if (previousPackets !== null && (previous === undefined || only.packetsReceived <= previous)) {
            return {
                timestamp: null,
                packetCounts,
                issue: "no_unique_active_stream",
                diagnostic: `${expectedKind}: sole RTP stream not advancing`,
            };
        }
        return {
            timestamp: only.timestamp,
            packetCounts,
            issue: null,
            diagnostic: `${expectedKind}: one advancing RTP timestamp candidate`,
        };
    }
    if (candidates.length > 1 && previousPackets) {
        const advancing = candidates.filter((candidate) => {
            const previous = previousPackets.get(candidate.id);
            return previous !== undefined && candidate.packetsReceived > previous;
        });
        if (advancing.length === 1) {
            return {
                timestamp: advancing[0]?.timestamp ?? null,
                packetCounts,
                issue: null,
                diagnostic: `${expectedKind}: selected sole advancing RTP stream`,
            };
        }
        if (advancing.length > 1) {
            return {
                timestamp: null,
                packetCounts,
                issue: "no_unique_active_stream",
                diagnostic: `${expectedKind}: ${advancing.length} advancing RTP streams`,
            };
        }
    }
    if (candidates.length === 0) {
        const issue = missingTimestamp > 0
            ? missingSenderReportMapping === missingTimestamp
                ? "sender_report_timing_unavailable"
                : "timestamp_unavailable"
            : "stats_unavailable";
        return {
            timestamp: null,
            packetCounts,
            issue,
            diagnostic: `${expectedKind}: inbound=${inboundForKind}, timestamp-missing=${missingTimestamp}, sender-report-timing-missing=${missingSenderReportMapping}, rtx=${excludedRtx}`,
        };
    }
    return {
        timestamp: null,
        packetCounts,
        issue: previousPackets ? "no_unique_active_stream" : "ambiguous_streams",
        diagnostic: `${expectedKind}: ${candidates.length} RTP timestamp candidates awaiting unique activity`,
    };
};
const requireAdvancingPlayoutClock = (selected, previouslyAccepted, kind) => {
    if (selected.timestamp === null || previouslyAccepted === null || selected.timestamp > previouslyAccepted) {
        return selected;
    }
    return {
        ...selected,
        timestamp: null,
        issue: "no_unique_active_stream",
        diagnostic: `${kind}: RTP packets advanced but estimated playout clock stalled or reset`,
    };
};
const mediaElementAvSyncFallback = (diagnostic, audioIssue, videoIssue) => {
    if (rt0EvidenceMode) {
        return {
            offsetMillis: null,
            reference: null,
            diagnostic: `${diagnostic}; strict-rt0=rtp-playout-timestamp-only`,
            audioIssue,
            videoIssue,
        };
    }
    const audioTime = avatarAudio.currentTime;
    const videoTime = video.currentTime;
    if (Number.isFinite(audioTime)
        && Number.isFinite(videoTime)
        && audioTime > 0
        && videoTime > 0) {
        return {
            offsetMillis: Math.round(Math.abs(audioTime - videoTime) * 1_000),
            reference: AV_SYNC_MEDIA_ELEMENT_REFERENCE,
            diagnostic: `${diagnostic}; fallback=media-element-current-time`,
            audioIssue: null,
            videoIssue: null,
        };
    }
    return {
        offsetMillis: null,
        reference: null,
        diagnostic: `${diagnostic}; media-element-current-time unavailable`,
        audioIssue,
        videoIssue,
    };
};
const readAvSyncOffsetMillis = async (state) => {
    const currentPeer = peer;
    let audioSelection;
    let videoSelection;
    if (currentPeer) {
        const stats = await currentPeer.getStats();
        audioSelection = selectPlayoutTimestamp(stats, "audio", state.audioPackets);
        videoSelection = selectPlayoutTimestamp(stats, "video", state.videoPackets);
    }
    else {
        const audioStats = liveKitAudioTrack?.getRTCStatsReport;
        const videoStats = liveKitVideoTrack?.getRTCStatsReport;
        if (!audioStats || !videoStats) {
            return mediaElementAvSyncFallback("LiveKit track stats method unavailable", "stats_unavailable", "stats_unavailable");
        }
        const [audioReport, videoReport] = await Promise.all([
            audioStats.call(liveKitAudioTrack),
            videoStats.call(liveKitVideoTrack),
        ]);
        audioSelection = selectPlayoutTimestamp(audioReport, "audio", state.audioPackets);
        videoSelection = selectPlayoutTimestamp(videoReport, "video", state.videoPackets);
    }
    audioSelection = requireAdvancingPlayoutClock(audioSelection, state.lastAudioPlayoutTimestamp, "audio");
    videoSelection = requireAdvancingPlayoutClock(videoSelection, state.lastVideoPlayoutTimestamp, "video");
    state.audioPackets = audioSelection.packetCounts;
    state.videoPackets = videoSelection.packetCounts;
    if (audioSelection.timestamp !== null)
        state.lastAudioPlayoutTimestamp = audioSelection.timestamp;
    if (videoSelection.timestamp !== null)
        state.lastVideoPlayoutTimestamp = videoSelection.timestamp;
    if (audioSelection.timestamp === null || videoSelection.timestamp === null) {
        return mediaElementAvSyncFallback(`${audioSelection.diagnostic}; ${videoSelection.diagnostic}`, audioSelection.issue, videoSelection.issue);
    }
    return {
        offsetMillis: Math.round(Math.abs(audioSelection.timestamp - videoSelection.timestamp)),
        reference: AV_SYNC_RTP_REFERENCE,
        diagnostic: "audio/video playout timestamps available",
        audioIssue: null,
        videoIssue: null,
    };
};
const collectAvSyncEvidence = async (requestSequence) => {
    let sampleSequence = 1;
    let attempts = 0;
    const readState = {
        audioPackets: null,
        videoPackets: null,
        lastAudioPlayoutTimestamp: null,
        lastVideoPlayoutTimestamp: null,
    };
    let lastAudioIssue = "stats_unavailable";
    let lastVideoIssue = "stats_unavailable";
    while (sampleSequence <= AV_SYNC_SAMPLE_COUNT && attempts < AV_SYNC_MAX_ATTEMPTS) {
        attempts += 1;
        let reading;
        try {
            reading = await readAvSyncOffsetMillis(readState);
        }
        catch {
            lastAudioIssue = "stats_unavailable";
            lastVideoIssue = "stats_unavailable";
            break;
        }
        if (reading.audioIssue !== null)
            lastAudioIssue = reading.audioIssue;
        if (reading.videoIssue !== null)
            lastVideoIssue = reading.videoIssue;
        if (reading.offsetMillis !== null && reading.reference !== null) {
            await api("/api/evidence/av-sync", {
                session_sequence: evidenceSessionSequence,
                request_sequence: requestSequence,
                sample_sequence: sampleSequence,
                reference: reading.reference,
                absolute_offset_millis: reading.offsetMillis,
            });
            sampleSequence += 1;
        }
        if (sampleSequence <= AV_SYNC_SAMPLE_COUNT && attempts < AV_SYNC_MAX_ATTEMPTS) {
            await new Promise((resolve) => window.setTimeout(resolve, AV_SYNC_SAMPLE_INTERVAL_MILLIS));
        }
    }
    if (sampleSequence <= AV_SYNC_SAMPLE_COUNT) {
        await api("/api/evidence/av-sync-diagnostic", {
            session_sequence: evidenceSessionSequence,
            request_sequence: requestSequence,
            attempts,
            audio_issue: lastAudioIssue,
            video_issue: lastVideoIssue,
        });
    }
    await refreshSessionEvidence();
};
const ensureAvSyncEvidence = (voice) => {
    if (!voice.responseComplete || voice.audioStartedElapsed === null)
        return null;
    if (!voice.avSyncEvidence) {
        avSyncCollectionPending = true;
        updateControls();
        const collection = collectAvSyncEvidence(voice.requestSequence).finally(() => {
            if (pendingAvSyncEvidence === collection)
                pendingAvSyncEvidence = null;
            avSyncCollectionPending = false;
            updateControls();
        });
        pendingAvSyncEvidence = collection;
        voice.avSyncEvidence = collection;
    }
    return voice.avSyncEvidence;
};
const ensurePlaybackCompletionEvidence = (voice) => {
    if (!voice.providerPlaybackDone || !voice.audioStarted || voice.interrupted)
        return null;
    if (!voice.playbackCompletionEvidence) {
        voice.playbackCompletionEvidence = (async () => {
            if (voice.audioStartedEvidence)
                await voice.audioStartedEvidence;
            await postMediaEvidence("playback_completed", performance.now() - voice.startedAt, voice.requestSequence);
            await refreshSessionEvidence();
        })();
    }
    return voice.playbackCompletionEvidence;
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
            voice.playbackSilenceStartedAt = null;
            if (!voice.audioStarted) {
                voice.audioStarted = true;
                voice.audioStartedElapsed = performance.now() - voice.startedAt;
                const audioStartedEvidence = postMediaEvidence("audio_started", voice.audioStartedElapsed, voice.requestSequence).then(async () => {
                    await syncStatus().catch(() => undefined);
                    const avSyncEvidence = ensureAvSyncEvidence(voice);
                    if (avSyncEvidence)
                        await avSyncEvidence;
                });
                voice.audioStartedEvidence = audioStartedEvidence;
                void audioStartedEvidence
                    .then(() => {
                    const completion = ensurePlaybackCompletionEvidence(voice);
                    if (completion)
                        void completion.catch(() => undefined);
                })
                    .catch(() => undefined);
            }
        }
        else if (voice?.speaking) {
            voice.silentFrames += 1;
            if (voice.silentFrames >= 6) {
                voice.speaking = false;
                voice.playbackSilenceStartedAt = performance.now();
            }
        }
        if (voice
            && rt0EvidenceMode
            && rt0PlaybackPending
            && voice.responseComplete
            && voice.audioStarted
            && !voice.providerPlaybackDone
            && !voice.interrupted
            && !voice.speaking
            && voice.playbackSilenceStartedAt !== null
            && !voice.playbackRecoveryTriggered
            && performance.now() - voice.playbackSilenceStartedAt
                >= UNCONFIRMED_PLAYBACK_SILENCE_RECOVERY_MILLIS) {
            voice.playbackRecoveryTriggered = true;
            void postMediaEvidence("playback_recovery_triggered", performance.now() - voice.startedAt, voice.requestSequence).catch(() => undefined);
            void interruptAvatar(false).then((recovered) => {
                if (recovered) {
                    setStatus("D-ID не подтвердил окончание playback. Зависший playback безопасно остановлен; "
                        + "следующий turn разблокирован, но playback_completed не засчитан.", "error");
                }
            });
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
        remoteEvidenceAudioContext = createRuntimeAudioContext();
    await remoteEvidenceAudioContext.resume();
    remoteAudioSource?.disconnect();
    remoteAudioAnalyser?.disconnect();
    remoteSilentGain?.disconnect();
    remoteAudioSource = remoteEvidenceAudioContext.createMediaStreamSource(createRuntimeMediaStream([track]));
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
    const now = performance.now();
    queueConnectionMediaEvidence("video_ready", now - connectEvidenceStartedAt);
    if (connectJourneyStartedAt > 0) {
        queueConnectionMediaEvidence("end_to_end_video_ready", now - connectJourneyStartedAt);
    }
    void connectionEvidenceTail
        .then(() => syncStatus())
        .catch(() => undefined);
};
const modalityLabel = (state) => {
    switch (state) {
        case "ready": return "Готов";
        case "preparing": return "Подготовка…";
        case "failed": return "Ошибка подготовки";
        default: return "Не готов";
    }
};
const renderModalityReadiness = (readiness) => {
    const entries = [
        [readinessText, readiness.text],
        [readinessVoice, readiness.voice],
        [readinessVideo, readiness.video],
    ];
    entries.forEach(([node, state]) => {
        node.textContent = modalityLabel(state);
        node.dataset.state = state;
    });
};
const syncStatus = () => {
    const run = statusSyncTail
        .catch(() => undefined)
        .then(async () => {
        const status = await api("/api/status");
        sessionState.applyBackend(status);
        if (status.session_state === "active" && evidenceSessionSequence === 0) {
            const snapshot = await runtimeFetch("/api/evidence/session", {
                method: "GET", credentials: "same-origin", cache: "no-store",
            }).then(async (response) => response.ok
                ? await response.json()
                : null).catch(() => null);
            observedBackendSessionSequence = snapshot
                && typeof snapshot.session_sequence === "number"
                && Number.isSafeInteger(snapshot.session_sequence)
                && snapshot.session_sequence > 0
                ? snapshot.session_sequence : 0;
        }
        if (evidenceSessionSequence > 0 && ["revoked", "closed"].includes(status.session_state)
            && locallyFencedSessionSequence !== evidenceSessionSequence) {
            broadcastSessionEgressFence();
            closePeerTransport();
        }
        renderModalityReadiness(status.modality_readiness);
        updateControls();
        showEvidence(status);
        return status;
    });
    statusSyncTail = run.then(() => undefined, () => undefined);
    return run;
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
const backendSessionPresent = () => !["none", "closed"].includes(sessionState.backend.session_state);
const selectedAudience = () => sessionState.backend.session_audience ?? audienceSelect.value;
const updateAudienceMode = () => {
    personaPanel.hidden = selectedAudience() === "visitor";
};
const updateControls = () => {
    const transportReady = sessionState.realtime.control && sessionState.backend.session_state === "active";
    const textReady = sessionState.backend.conversation_readiness !== "none";
    const voiceReady = sessionState.backend.conversation_readiness === "text_and_voice";
    const playbackReady = activeClientControl?.interrupt_requires_playback_id
        ? sessionState.playbackId !== null
        : true;
    const clientInterruptReady = transportReady
        && activeClientControl?.interrupt === true
        && playbackReady
        && providerPlaybackInFlight;
    const strictPlaybackBlocked = rt0EvidenceMode && rt0PlaybackPending;
    speakButton.disabled = !transportReady || !textReady || textRequestInFlight || voiceRequestInFlight || strictPlaybackBlocked;
    interruptButton.disabled = !voiceRequestInFlight && !clientInterruptReady;
    resumeAnswerButton.disabled = resumeAnswerRow.hidden || !transportReady
        || voiceRequestInFlight || textRequestInFlight || recording || !liveKitRoom;
    voiceButton.disabled = recording
        ? false
        : !transportReady || !voiceReady || textRequestInFlight || voiceRequestInFlight || strictPlaybackBlocked;
    voiceButton.textContent = recording ? "Остановить и отправить" : "Начать говорить";
    revokeButton.disabled = !backendSessionPresent()
        || (sessionState.backend.session_state === "revoked" && !sessionState.backend.avatar_open);
    closeButton.disabled = !backendSessionPresent()
        || (rt0EvidenceMode && avSyncCollectionPending);
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
        if (bootstrapComplete) {
            void syncStatus().catch(() => undefined);
        }
    },
});
const handleProviderClientEvent = (raw) => {
    if (!raw)
        return;
    void api("/api/avatar/client-event", { message: raw })
        .then(async (normalized) => {
        if (normalized?.kind === "playback_started") {
            sessionState.setPlaybackId(normalized.playback_id);
            providerPlaybackInFlight = true;
        }
        else if (normalized?.kind === "playback_done") {
            const voice = activeVoiceEvidence;
            sessionState.setPlaybackId(null);
            voiceCommandScheduler.playbackDone();
            if (voice) {
                voice.providerPlaybackDoneCount += 1;
                const completion = maybeFinalizeProviderPlayback(voice);
                syncRt0PlaybackPending(voice);
                providerPlaybackInFlight = !voice.interrupted && !voice.providerPlaybackDone;
                updateControls();
                await completion;
            }
            else {
                providerPlaybackInFlight = voiceCommandScheduler.hasPendingPlayback;
                if (rt0EvidenceMode)
                    rt0PlaybackPending = false;
            }
        }
        else if (normalized) {
            const voice = activeVoiceEvidence;
            if (voice && !voice.providerEventKindsPosted.has(normalized.kind)) {
                const kind = {
                    video_generation_started: "provider_video_generation_started",
                    video_generation_done: "provider_video_generation_done",
                    video_generation_failed: "provider_video_generation_failed",
                    informational: "provider_informational_event",
                    unknown_chat_event: "provider_unknown_chat_event",
                    unknown_video_event: "provider_unknown_video_event",
                    unknown_tool_event: "provider_unknown_tool_event",
                    unknown_other_event: "provider_unknown_other_event",
                }[normalized.kind];
                const saved = await postMediaEvidence(kind, performance.now() - voice.startedAt, voice.requestSequence)
                    .then(() => true, () => false);
                if (saved)
                    voice.providerEventKindsPosted.add(normalized.kind);
            }
        }
        else if (normalized === null) {
            const voice = activeVoiceEvidence;
            if (voice && !voice.providerIgnoredEventPosted) {
                const saved = await postMediaEvidence("provider_event_ignored", performance.now() - voice.startedAt, voice.requestSequence).then(() => true, () => false);
                if (saved)
                    voice.providerIgnoredEventPosted = true;
            }
        }
        updateControls();
    })
        .catch(async () => {
        const voice = activeVoiceEvidence;
        if (voice && !voice.providerParseFailurePosted) {
            voice.providerParseFailurePosted = true;
            await postMediaEvidence("provider_event_parse_failed", performance.now() - voice.startedAt, voice.requestSequence).catch(() => undefined);
        }
    });
};
const dispatchClientCommand = async (command) => {
    if (evidenceSessionSequence > 0
        && locallyFencedSessionSequence === evidenceSessionSequence) {
        throw new Error("SESSION_EGRESS_FENCED");
    }
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
const syncRt0PlaybackPending = (voice) => {
    if (!rt0EvidenceMode)
        return;
    rt0PlaybackPending = voice !== null
        && !voice.interrupted
        && !voice.providerPlaybackDone
        && voice.providerPlaybackExpectedCount > 0
        && voice.providerPlaybackDoneCount !== voice.providerPlaybackExpectedCount;
};
const maybeFinalizeProviderPlayback = async (voice) => {
    if (voice.providerPlaybackDone
        || voice.interrupted
        || !voice.responseComplete
        || voice.providerPlaybackExpectedCount === 0
        || voice.providerPlaybackDoneCount !== voice.providerPlaybackExpectedCount
        || voiceCommandScheduler.hasPendingPlayback) {
        return;
    }
    voice.providerPlaybackDone = true;
    voice.playbackSilenceStartedAt = null;
    syncRt0PlaybackPending(voice);
    if (!voice.providerPlaybackDoneEvidencePosted) {
        voice.providerPlaybackDoneEvidencePosted = true;
        await postMediaEvidence("provider_playback_done_received", performance.now() - voice.startedAt, voice.requestSequence);
    }
    const completion = ensurePlaybackCompletionEvidence(voice);
    if (completion)
        await completion;
};
const attachLiveKitTrack = (track) => {
    if (track.kind === "video") {
        liveKitVideoTrack = track;
        if (!videoTrackReceivedEvidencePosted && connectJourneyStartedAt > 0) {
            videoTrackReceivedEvidencePosted = true;
            queueConnectionMediaEvidence("remote_video_track_received", performance.now() - connectJourneyStartedAt);
        }
        track.attach(video);
        if (!videoAttachedEvidencePosted && connectJourneyStartedAt > 0) {
            videoAttachedEvidencePosted = true;
            queueConnectionMediaEvidence("remote_video_attached", performance.now() - connectJourneyStartedAt);
        }
        sessionState.setRealtimeReadiness({ video: true });
        stage?.classList.add("has-video");
        if (!requestVideoFrame(video, recordFirstVideoFrame)) {
            video.addEventListener("playing", recordFirstVideoFrame, { once: true });
        }
        setStatus("Видео подключено", "ready");
        updateControls();
    }
    else if (track.kind === "audio") {
        liveKitAudioTrack = track;
        sessionState.setRealtimeReadiness({ audio: true });
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
    setMediaSrcObject(element, null);
};
const handleLiveKitTrackUnsubscribed = (track) => {
    if (track === liveKitAudioTrack) {
        detachLiveKitTrack(track, avatarAudio);
        liveKitAudioTrack = null;
        sessionState.setRealtimeReadiness({ audio: false });
        setStatus("Аудиопоток аватара потерян. Микрофон и текст остаются доступны; ожидаю восстановление LiveKit…", "error");
        if (voiceRequestInFlight || voiceCommandScheduler.hasActivePlayback || providerPlaybackInFlight) {
            void interruptAvatar();
        }
    }
    if (track === liveKitVideoTrack) {
        detachLiveKitTrack(track, video);
        liveKitVideoTrack = null;
        sessionState.setRealtimeReadiness({ video: false });
        stage?.classList.remove("has-video");
        setStatus("Видео-поток аватара потерян. Голос остаётся доступен; ожидаю восстановление LiveKit…", "error");
    }
    updateControls();
};
const clearRealtimeMedia = () => {
    sessionState.resetRealtime();
    liveKitAudioTrack = null;
    liveKitVideoTrack = null;
    setMediaSrcObject(video, null);
    setMediaSrcObject(avatarAudio, null);
    stage?.classList.remove("has-video");
    updateControls();
};
const closePeerTransport = () => {
    clearInterruptedAnswer();
    voiceDeliveryGeneration += 1;
    voiceCommandScheduler.interrupt();
    rt0PlaybackPending = false;
    providerPlaybackInFlight = false;
    stopMicrophoneCapture();
    stopRemoteEvidence();
    providerDataChannel?.close();
    providerDataChannel = null;
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
const sessionEndRequest = () => {
    const sequence = evidenceSessionSequence > 0
        ? evidenceSessionSequence : observedBackendSessionSequence;
    return sequence > 0 ? { expected_session_sequence: sequence } : {};
};
const broadcastSessionEgressFence = () => {
    const sequence = evidenceSessionSequence > 0
        ? evidenceSessionSequence : observedBackendSessionSequence;
    if (sequence <= 0)
        return;
    locallyFencedSessionSequence = sequence;
    sessionEgressFence?.postMessage({
        kind: "session-egress-revoked",
        evidence_session_sequence: sequence,
    });
};
sessionEgressFence?.addEventListener("message", (event) => {
    const signal = event.data;
    if (!signal || signal.kind !== "session-egress-revoked"
        || typeof signal.evidence_session_sequence !== "number"
        || !Number.isSafeInteger(signal.evidence_session_sequence)
        || signal.evidence_session_sequence !== (evidenceSessionSequence > 0
            ? evidenceSessionSequence : observedBackendSessionSequence)
        || signal.evidence_session_sequence <= 0)
        return;
    locallyFencedSessionSequence = signal.evidence_session_sequence;
    closePeerTransport();
    setStatus("Доступ отозван в другой вкладке. Отправка речи остановлена.", "error");
    updateControls();
});
const waitForCanonicalTerminal = async (states, timeoutMillis) => {
    const deadline = performance.now() + timeoutMillis;
    do {
        const status = await syncStatus().catch(() => null);
        if (status && states.has(status.session_state))
            return true;
        if (performance.now() >= deadline)
            return false;
        await new Promise((resolve) => window.setTimeout(resolve, 250));
    } while (true);
};
const handleUnexpectedLiveKitDisconnect = async (room, reason) => {
    if (liveKitRoom !== room)
        return;
    const reasonSuffix = reason === undefined ? "" : ` (reason=${String(reason)})`;
    broadcastSessionEgressFence();
    closePeerTransport();
    setStatus(`LiveKit отключен${reasonSuffix}. Завершаю зависшую сессию…`, "error");
    if (!backendSessionPresent())
        return;
    const evidenceFlush = tryFlushConnectionMediaEvidence();
    let revokeError = null;
    try {
        await api("/api/session/revoke", sessionEndRequest());
    }
    catch (error) {
        revokeError = error;
    }
    const connectionEvidenceError = await evidenceFlush.catch((error) => error instanceof Error ? error : new Error(String(error)));
    const evidenceWarning = connectionEvidenceError
        ? `; connection evidence incomplete: ${connectionEvidenceError.message}`
        : "";
    try {
        await api("/api/session/close", sessionEndRequest());
        await syncStatus();
        await refreshSessionEvidence();
        try {
            await downloadSessionEvidence();
            setStatus(`LiveKit отключен${reasonSuffix}. Сессия закрыта, evidence snapshot сохранён${evidenceWarning}${revokeError ? "; первоначальный отзыв потребовал повторного закрытия" : ""}. Подключитесь снова.`, "error");
        }
        catch (exportError) {
            setStatus(exportError instanceof Error
                ? `LiveKit отключен${reasonSuffix}. Сессия закрыта${evidenceWarning}; evidence export: ${exportError.message}`
                : `LiveKit отключен${reasonSuffix}. Сессия закрыта${evidenceWarning}; evidence export failed`, "error");
        }
    }
    catch (error) {
        const terminal = await waitForCanonicalTerminal(new Set(["closed"]), 45_000);
        if (terminal && sessionState.backend.avatar_open === false) {
            await refreshSessionEvidence().catch(() => undefined);
            try {
                await downloadSessionEvidence();
                setStatus(`LiveKit отключен${reasonSuffix}. Сессия закрыта, evidence snapshot сохранён${evidenceWarning}. Подключитесь снова.`, "error");
            }
            catch (exportError) {
                setStatus(exportError instanceof Error
                    ? `LiveKit отключен${reasonSuffix}. Сессия закрыта; evidence export: ${exportError.message}`
                    : `LiveKit отключен${reasonSuffix}. Сессия закрыта; evidence export failed`, "error");
            }
        }
        else {
            setStatus(error instanceof Error
                ? `LiveKit отключен${reasonSuffix}; canonical cleanup pending: ${error.message}`
                : `LiveKit отключен${reasonSuffix}; canonical cleanup pending`, "error");
        }
    }
};
const connectWebRtcTransport = async (transport, clientControl) => {
    peer = createRuntimePeerConnection({
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
            sessionState.setPlaybackId(null);
            updateControls();
        };
        channel.onmessage = (event) => {
            const raw = typeof event.data === "string" ? event.data : "";
            handleProviderClientEvent(raw);
        };
    }
    peer.ontrack = (event) => {
        remoteMediaStream ??= createRuntimeMediaStream();
        if (!remoteMediaStream.getTracks().some((track) => track.id === event.track.id)) {
            remoteMediaStream.addTrack(event.track);
        }
        setMediaSrcObject(video, remoteMediaStream);
        if (event.track.kind === "video") {
            if (!videoTrackReceivedEvidencePosted && connectJourneyStartedAt > 0) {
                videoTrackReceivedEvidencePosted = true;
                queueConnectionMediaEvidence("remote_video_track_received", performance.now() - connectJourneyStartedAt);
            }
            if (!videoAttachedEvidencePosted && connectJourneyStartedAt > 0) {
                videoAttachedEvidencePosted = true;
                queueConnectionMediaEvidence("remote_video_attached", performance.now() - connectJourneyStartedAt);
            }
            sessionState.setRealtimeReadiness({ video: true });
            stage?.classList.add("has-video");
            if (!requestVideoFrame(video, recordFirstVideoFrame)) {
                video.addEventListener("playing", recordFirstVideoFrame, { once: true });
            }
            setStatus("Видео подключено", "ready");
        }
        else if (event.track.kind === "audio") {
            sessionState.setRealtimeReadiness({ audio: true });
            void attachRemoteAudioEvidence(event.track).catch(() => undefined);
        }
        updateControls();
    };
    peer.onconnectionstatechange = () => {
        if (!peer)
            return;
        const state = peer.connectionState;
        sessionState.setRealtimeReadiness({ control: state === "connected" });
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
        const voice = activeVoiceEvidence;
        if (voice && !voice.providerDataReceived) {
            voice.providerDataReceived = true;
            void postMediaEvidence("provider_data_received", performance.now() - voice.startedAt, voice.requestSequence).catch(() => undefined);
        }
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
    sessionState.setRealtimeReadiness({ control: true });
    updateControls();
};
const connectAvatar = async () => {
    if (!consent.checked) {
        setStatus("Нужно явное согласие", "error");
        return;
    }
    connectButton.disabled = true;
    connectEvidenceStartedAt = 0;
    connectJourneyStartedAt = performance.now();
    resetTelemetry();
    videoEvidencePosted = false;
    videoTrackReceivedEvidencePosted = false;
    videoAttachedEvidencePosted = false;
    reconnectStartedAt = null;
    connectionEvidenceTail = Promise.resolve();
    connectionEvidenceFailure = null;
    evidenceSessionSequence = 0;
    nextTextRequestSequence = 0;
    nextVoiceRequestSequence = 0;
    setStatus("Создаю защищённую сессию…");
    let backendSessionStarted = false;
    try {
        const audience = audienceSelect.value;
        const start = await api("/api/avatar/start", { consent: true, audience });
        backendSessionStarted = true;
        const backendReadyAt = performance.now();
        evidenceSessionSequence = start.evidence_session_sequence;
        observedBackendSessionSequence = start.evidence_session_sequence;
        locallyFencedSessionSequence = 0;
        queueConnectionMediaEvidence("backend_start_ready", backendReadyAt - connectJourneyStartedAt);
        capabilities = new Set(start.capabilities);
        activeClientControl = start.client_control;
        const startedStatus = await syncStatus();
        if (startedStatus.session_state !== "active"
            || startedStatus.avatar_open !== true
            || startedStatus.session_audience !== audience) {
            throw new Error("SESSION_START_STATE_MISMATCH");
        }
        queueConnectionMediaEvidence("transport_connect_started", performance.now() - connectJourneyStartedAt);
        if (start.transport.kind === "web_rtc") {
            await connectWebRtcTransport(start.transport, start.client_control);
        }
        else {
            await connectLiveKitTransport(start.transport);
        }
        connectEvidenceStartedAt = performance.now();
        queueConnectionMediaEvidence("transport_connected", connectEvidenceStartedAt - connectJourneyStartedAt);
        if (!videoEvidencePosted) {
            if (video.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA) {
                recordFirstVideoFrame();
            }
            else if (!requestVideoFrame(video, recordFirstVideoFrame)) {
                video.addEventListener("playing", recordFirstVideoFrame, { once: true });
            }
        }
        ensureMicrophoneDeviceMonitoring();
        await refreshMicrophoneDevices(storedMicrophoneDeviceId());
        await syncStatus();
        const transportName = start.transport.kind === "web_rtc" ? "WebRTC" : "LiveKit";
        setStatus(selectedAudience() === "visitor"
            ? `Visitor-сессия ${transportName} согласована`
            : `${transportName} согласован`, "ready");
        showEvidence({ transport: start.transport.kind, capabilities: [...capabilities] });
    }
    catch (error) {
        const messageText = error instanceof Error ? error.message : "Ошибка подключения";
        const connectionEvidenceError = await tryFlushConnectionMediaEvidence();
        const evidenceWarning = connectionEvidenceError
            ? `; connection evidence incomplete: ${connectionEvidenceError.message}`
            : "";
        closePeerTransport();
        if (backendSessionStarted || backendSessionPresent()) {
            try {
                await api("/api/session/close", sessionEndRequest());
                backendSessionStarted = false;
                await syncStatus();
            }
            catch (cleanupError) {
                await syncStatus().catch(() => undefined);
                const cleanupText = cleanupError instanceof Error ? cleanupError.message : "cleanup failed";
                setStatus(`${messageText}${evidenceWarning}; cleanup: ${cleanupText}`, "error");
                updateControls();
                return;
            }
        }
        setStatus(`${messageText}${evidenceWarning}`, "error");
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
    microphoneLevel.value = 0;
    microphoneLevelText.textContent = "Сигнал появится во время записи.";
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
const storedMicrophoneDeviceId = () => {
    try {
        return window.localStorage.getItem(MICROPHONE_STORAGE_KEY)?.trim() ?? "";
    }
    catch {
        return "";
    }
};
const rememberMicrophoneDeviceId = (deviceId) => {
    try {
        if (deviceId)
            window.localStorage.setItem(MICROPHONE_STORAGE_KEY, deviceId);
        else
            window.localStorage.removeItem(MICROPHONE_STORAGE_KEY);
    }
    catch {
    }
};
const refreshMicrophoneDevices = async (preferredDeviceId) => {
    const mediaDevices = runtimeMediaDevices();
    if (!mediaDevices?.enumerateDevices)
        return;
    let devices;
    try {
        devices = (await mediaDevices.enumerateDevices())
            .filter((device) => device.kind === "audioinput");
    }
    catch {
        return;
    }
    const requested = (preferredDeviceId ?? microphoneSelect.value ?? storedMicrophoneDeviceId()).trim();
    microphoneSelect.replaceChildren();
    microphoneSelect.add(new Option("Системный микрофон по умолчанию", ""));
    devices.forEach((device, index) => {
        microphoneSelect.add(new Option(device.label || `Микрофон ${index + 1}`, device.deviceId));
    });
    if (requested && devices.some((device) => device.deviceId === requested)) {
        microphoneSelect.value = requested;
    }
    else {
        microphoneSelect.value = "";
        if (requested)
            rememberMicrophoneDeviceId("");
    }
};
const ensureMicrophoneDeviceMonitoring = () => {
    if (microphoneDeviceListenerInstalled)
        return;
    const mediaDevices = runtimeMediaDevices();
    mediaDevices?.addEventListener?.("devicechange", () => {
        if (backendSessionPresent())
            void refreshMicrophoneDevices();
    });
    microphoneDeviceListenerInstalled = true;
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
    const mediaDevices = runtimeMediaDevices();
    if (!mediaDevices?.getUserMedia)
        throw new Error("MIC_UNAVAILABLE");
    try {
        const selectedDeviceId = microphoneSelect.value.trim();
        micStream = await mediaDevices.getUserMedia({
            audio: {
                ...(selectedDeviceId ? { deviceId: { exact: selectedDeviceId } } : {}),
                channelCount: 1,
                sampleRate: 16_000,
                echoCancellation: true,
                noiseSuppression: true,
                autoGainControl: true,
            },
        });
        const activeDeviceId = micStream.getAudioTracks()[0]?.getSettings().deviceId ?? selectedDeviceId;
        await refreshMicrophoneDevices(activeDeviceId);
        if (activeDeviceId) {
            microphoneSelect.value = activeDeviceId;
            rememberMicrophoneDeviceId(activeDeviceId);
        }
    }
    catch (error) {
        throw microphoneCaptureError(error);
    }
    audioContext = createRuntimeAudioContext({ sampleRate: 16_000, latencyHint: "interactive" });
    if (audioContext.sampleRate !== 16_000) {
        stopMicrophoneCapture();
        throw new Error("MIC_SAMPLE_RATE_UNSUPPORTED");
    }
    await audioContext.audioWorklet.addModule("/mic-worklet.js");
    micSource = audioContext.createMediaStreamSource(micStream);
    micWorklet = createRuntimeAudioWorkletNode(audioContext, "vpr-mic-capture");
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
        let energy = 0;
        for (const sample of samples)
            energy += sample * sample;
        const rms = samples.length === 0 ? 0 : Math.sqrt(energy / samples.length);
        microphoneLevel.value = Math.min(1, rms * 8);
        microphoneLevelText.textContent = rms < 0.001
            ? "Сигнал почти нулевой — браузер не получает слышимый звук с выбранного микрофона."
            : `Сигнал есть · RMS ${rms.toFixed(4)}`;
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
    clearInterruptedAnswer();
    resumeSourceRequest = requestSequence;
    voiceRequestInFlight = true;
    const attemptedRequestSequence = requestSequence;
    const deliveryGeneration = ++voiceDeliveryGeneration;
    let finishAccepted = false;
    updateControls();
    setStatus("Завершаю распознавание и начинаю ответ…");
    activeVoiceEvidence = {
        requestSequence,
        startedAt: performance.now(),
        audioStarted: false,
        audioStartedElapsed: null,
        audioStartedEvidence: null,
        avSyncEvidence: null,
        playbackCompletionEvidence: null,
        providerDataReceived: false,
        providerIgnoredEventPosted: false,
        providerEventKindsPosted: new Set(),
        providerParseFailurePosted: false,
        providerPlaybackExpectedCount: 0,
        providerPlaybackDoneCount: 0,
        providerPlaybackDoneEvidencePosted: false,
        providerPlaybackDone: false,
        responseComplete: false,
        interrupted: false,
        speaking: false,
        silentFrames: 0,
        playbackSilenceStartedAt: null,
        playbackRecoveryTriggered: false,
    };
    let terminalStatus = null;
    try {
        const started = await apiEvidenceJson("/api/voice/input/finish", {}, requestSequence);
        if (started.request_sequence !== requestSequence)
            throw new Error("VOICE_STREAM_SEQUENCE_MISMATCH");
        finishAccepted = true;
        let deliveryFailure = null;
        let clientDeliverySentElapsed = null;
        const deliveryTasks = [];
        let liveKitSendTail = Promise.resolve();
        const scheduleSegmentDelivery = (segment) => {
            if (deliveryGeneration !== voiceDeliveryGeneration)
                return;
            const voiceForSegment = activeVoiceEvidence;
            if (voiceForSegment?.requestSequence === requestSequence) {
                voiceForSegment.providerPlaybackExpectedCount += 1;
                providerPlaybackInFlight = true;
                syncRt0PlaybackPending(voiceForSegment);
                updateControls();
            }
            const command = segment.client_command;
            if (!command)
                return;
            let commandSent = false;
            const nativeLiveKitQueue = command.route.kind === "live_kit_text_topic";
            const dispatch = nativeLiveKitQueue
                ? (() => {
                    const generation = voiceDeliveryGeneration;
                    const send = liveKitSendTail.then(async () => {
                        if (generation !== voiceDeliveryGeneration)
                            return false;
                        await dispatchClientCommand(command);
                        return true;
                    });
                    liveKitSendTail = send.then(() => undefined, () => undefined);
                    return send;
                })()
                : voiceCommandScheduler.dispatch(command);
            const task = (async () => {
                const sent = await dispatch;
                if (!sent) {
                    syncRt0PlaybackPending(activeVoiceEvidence);
                    updateControls();
                    return;
                }
                commandSent = true;
                const voice = activeVoiceEvidence;
                const clientDeliveryElapsed = voice?.requestSequence === requestSequence
                    && clientDeliverySentElapsed === null
                    ? performance.now() - voice.startedAt
                    : null;
                if (clientDeliveryElapsed !== null) {
                    clientDeliverySentElapsed = clientDeliveryElapsed;
                }
                await api("/api/avatar/client-delivery-sent", {
                    evidence_turn_sequence: segment.evidence_turn_sequence,
                    evidence_output_sequence: segment.evidence_output_sequence,
                });
                if (nativeLiveKitQueue && resumeSourceRequest === requestSequence
                    && authorizedDeliveredParts.size < 32) {
                    const part = authorizedSpeakText(command);
                    if (part)
                        authorizedDeliveredParts.set(segment.evidence_output_sequence, part);
                }
                if (clientDeliveryElapsed !== null) {
                    const evidence = postMediaEvidence("client_delivery_sent", clientDeliveryElapsed, requestSequence);
                    if (rt0EvidenceMode)
                        await evidence;
                    else
                        void evidence.catch(() => undefined);
                }
            })().catch(async (error) => {
                deliveryFailure = error instanceof Error ? error : new Error("CLIENT_TRANSPORT_UNAVAILABLE");
                if (commandSent) {
                    const providerInterrupted = await interruptAvatar(false);
                    if (rt0EvidenceMode && !providerInterrupted) {
                        rt0PlaybackPending = true;
                        updateControls();
                    }
                }
                else {
                    voiceDeliveryGeneration += 1;
                    voiceCommandScheduler.interrupt();
                    rt0PlaybackPending = false;
                    updateControls();
                    await api("/api/avatar/interrupt", {}).catch(() => undefined);
                }
                if (activeVoiceEvidence?.requestSequence === requestSequence) {
                    setStatus(deliveryFailure.message, "error");
                }
                throw deliveryFailure;
            });
            deliveryTasks.push(task);
            if (!rt0EvidenceMode)
                void task.catch(() => undefined);
        };
        const result = await waitForVoiceEvents(requestSequence, scheduleSegmentDelivery);
        const voiceAtBackendComplete = activeVoiceEvidence;
        const backendCompleteElapsed = voiceAtBackendComplete?.requestSequence === requestSequence
            ? performance.now() - voiceAtBackendComplete.startedAt
            : null;
        if (backendCompleteElapsed !== null) {
            const evidence = postMediaEvidence("backend_complete_received", backendCompleteElapsed, requestSequence);
            if (rt0EvidenceMode)
                await evidence;
            else
                void evidence.catch(() => undefined);
        }
        if (rt0EvidenceMode) {
            await Promise.all(deliveryTasks);
        }
        if (deliveryFailure)
            throw deliveryFailure;
        const voice = activeVoiceEvidence;
        if (voice?.requestSequence === requestSequence) {
            voice.responseComplete = true;
            if (authorizedDeliveredParts.size > 0 && result.reply.trim().length <= 16_000)
                completedAuthorizedReply = result.reply.trim();
            await maybeFinalizeProviderPlayback(voice);
            if (voice.audioStartedEvidence) {
                await voice.audioStartedEvidence;
            }
            const avSyncEvidence = ensureAvSyncEvidence(voice);
            if (avSyncEvidence)
                await avSyncEvidence;
        }
        await refreshSessionEvidence();
        terminalStatus = {
            text: `Вы: ${result.transcript} · Ответ: ${result.reply}`,
            kind: "ready",
        };
    }
    catch (error) {
        if (!finishAccepted) {
            await apiEvidenceJson("/api/voice/input/cancel", {}, requestSequence).catch(() => undefined);
        }
        if (activeVoiceEvidence?.requestSequence === attemptedRequestSequence)
            activeVoiceEvidence = null;
        await refreshSessionEvidence();
        terminalStatus = {
            text: error instanceof Error ? error.message : "Ошибка голосового запроса",
            kind: "error",
        };
    }
    finally {
        resetMicrophoneUpload();
        voiceRequestInFlight = false;
        updateControls();
        if (terminalStatus && sessionState.backend.session_state === "active"
            && locallyFencedSessionSequence !== evidenceSessionSequence) {
            setStatus(terminalStatus.text, terminalStatus.kind);
        }
    }
};
const toggleVoice = async () => {
    try {
        if (recording) {
            await finishMicrophoneTurn();
        }
        else {
            if (rt0EvidenceMode && rt0PlaybackPending) {
                throw new Error("RT0_EVIDENCE_PLAYBACK_ACTIVE_USE_INTERRUPT");
            }
            clearInterruptedAnswer();
            if (voiceCommandScheduler.hasActivePlayback || providerPlaybackInFlight) {
                if (!await interruptAvatar()) {
                    throw new Error("PROVIDER_STOP_UNCONFIRMED");
                }
            }
            await startMicrophone();
        }
    }
    catch (error) {
        stopMicrophoneCapture();
        await cancelMicrophoneInput();
        if (sessionState.backend.session_state === "active") {
            setStatus(error instanceof Error ? error.message : "Ошибка микрофона", "error");
        }
    }
};
const speak = async () => {
    const text = message.value.trim();
    if (!text)
        return;
    clearInterruptedAnswer();
    if (rt0EvidenceMode && rt0PlaybackPending) {
        setStatus("RT0 evidence: дождитесь окончания текущего playback или нажмите «Прервать».", "error");
        return;
    }
    const requestSequence = ++nextTextRequestSequence;
    textRequestInFlight = true;
    updateControls();
    let terminalStatus = null;
    try {
        const result = await apiEvidenceJson("/api/text/turn", { text }, requestSequence);
        message.value = "";
        await refreshSessionEvidence();
        terminalStatus = { text: `Ответ: ${result.reply}`, kind: "ready" };
    }
    catch (error) {
        terminalStatus = {
            text: error instanceof Error ? error.message : "Ошибка текстового разговора",
            kind: "error",
        };
    }
    finally {
        textRequestInFlight = false;
        updateControls();
        if (terminalStatus && sessionState.backend.session_state === "active"
            && locallyFencedSessionSequence !== evidenceSessionSequence) {
            setStatus(terminalStatus.text, terminalStatus.kind);
        }
    }
};
const interruptAvatar = async (recordEvidence = true) => {
    const voice = activeVoiceEvidence;
    if (voice)
        voice.interrupted = true;
    if (recordEvidence && voice?.audioStarted) {
        interruptEvidenceWatch = {
            requestSequence: voice.requestSequence,
            startedAt: performance.now(),
            silentFrames: 0,
        };
    }
    const playbackId = sessionState.playbackId;
    const playbackReady = activeClientControl?.interrupt_requires_playback_id
        ? playbackId !== null
        : true;
    const clientReady = !textRequestInFlight
        && sessionState.realtime.control
        && activeClientControl?.interrupt === true
        && playbackReady;
    const preparedInterrupt = activeClientControl?.prepared_interrupt ?? null;
    const providerStopRequired = providerPlaybackInFlight || voiceCommandScheduler.hasPendingPlayback;
    voiceDeliveryGeneration += 1;
    voiceCommandScheduler.interrupt();
    providerPlaybackInFlight = false;
    const fastProviderStop = clientReady && preparedInterrupt
        ? dispatchClientCommand(preparedInterrupt)
        : null;
    try {
        if (providerStopRequired && !clientReady) {
            throw new Error("PROVIDER_STOP_UNAVAILABLE");
        }
        const canonicalStop = voiceRequestInFlight
            ? api("/api/avatar/interrupt", {})
            : null;
        void canonicalStop?.catch(() => undefined);
        if (fastProviderStop) {
            await fastProviderStop;
            if (canonicalStop)
                await canonicalStop;
            rt0PlaybackPending = false;
            sessionState.setPlaybackId(null);
            updateControls();
            await refreshSessionEvidence();
            return true;
        }
        if (canonicalStop)
            await canonicalStop;
        if (clientReady) {
            const command = await api("/api/avatar/client-interrupt", {
                playback_id: playbackId,
            });
            await dispatchClientCommand(command);
            rt0PlaybackPending = false;
            sessionState.setPlaybackId(null);
            updateControls();
            await refreshSessionEvidence();
            return true;
        }
        if (!voiceRequestInFlight) {
            await api("/api/avatar/interrupt", {});
            rt0PlaybackPending = false;
            updateControls();
            await refreshSessionEvidence();
            return true;
        }
        await refreshSessionEvidence();
        return false;
    }
    catch (error) {
        interruptEvidenceWatch = null;
        broadcastSessionEgressFence();
        closePeerTransport();
        let cleanupFailed = false;
        try {
            await api("/api/session/revoke", sessionEndRequest());
        }
        catch {
            cleanupFailed = true;
        }
        if (cleanupFailed) {
            setStatus("PROVIDER_STOP_UNCONFIRMED: canonical cleanup pending", "error");
            await waitForCanonicalTerminal(new Set(["revoked", "closed"]), 45_000);
        }
        else {
            await syncStatus().catch(() => undefined);
        }
        const canonicalRevoked = ["revoked", "closed"].includes(sessionState.backend.session_state);
        const cause = error instanceof Error ? error.message : "PROVIDER_STOP_UNCONFIRMED";
        setStatus(!canonicalRevoked
            ? `PROVIDER_STOP_UNCONFIRMED_REVOKE_FAILED: ${cause}`
            : cleanupFailed
                ? `PROVIDER_STOP_UNCONFIRMED_CANONICAL_REVOKED_CLEANUP_PENDING: ${cause}`
                : `PROVIDER_STOP_UNCONFIRMED_SESSION_REVOKED: ${cause}`, "error");
        updateControls();
        return false;
    }
};
const interruptAndOfferResume = async () => {
    const voice = activeVoiceEvidence;
    const replaying = resumedSpeechStartedAt !== null;
    const eligible = sessionState.backend.session_state === "active" && !!liveKitRoom && (replaying || (voice?.responseComplete === true && voice.audioStarted && !voice.interrupted
        && resumeSourceRequest === voice.requestSequence && completedAuthorizedReply !== null));
    const sentences = replaying
        ? interruptedAnswerSentences
        : resumeSentences(completedAuthorizedReply ? [completedAuthorizedReply] : []);
    const elapsed = replaying
        ? performance.now() - (resumedSpeechStartedAt ?? performance.now())
        : voice?.audioStartedElapsed === null || voice?.audioStartedElapsed === undefined
            ? 0
            : performance.now() - voice.startedAt - voice.audioStartedElapsed;
    const offset = replaying ? resumeFromIndex : 0;
    const interrupted = await interruptAvatar();
    resumedSpeechStartedAt = null;
    if (interrupted && eligible && sentences.length > 0 && sessionState.backend.session_state === "active") {
        try {
            if (resumeSourceRequest === null)
                throw new Error("RESUME_SOURCE_UNAVAILABLE");
            await api("/api/avatar/confirm-interruption", {
                request_sequence: resumeSourceRequest,
            });
            offerInterruptedAnswer(sentences, elapsed, offset);
            setStatus("Ответ остановлен. Можно продолжить сохранённый текст с выбранного предложения. "
                + "Точное слово остановки D-ID не сообщает.", "ready");
        }
        catch (error) {
            clearInterruptedAnswer();
            setStatus(error instanceof Error ? `Продолжение недоступно: ${error.message}`
                : "Продолжение не подтверждено сервером.", "error");
        }
    }
    else {
        clearInterruptedAnswer();
        if (interrupted) {
            setStatus("Ответ остановлен. Продолжение недоступно: исходный ответ ещё не сформирован полностью.", "idle");
        }
    }
    updateControls();
};
const resumeInterruptedAnswer = async () => {
    if (resumeAnswerRow.hidden || sessionState.backend.session_state !== "active" || !liveKitRoom
        || !sessionState.realtime.control || recording || voiceRequestInFlight || textRequestInFlight) {
        return;
    }
    const index = Number(resumeAnswerFrom.value);
    const text = replayTextFrom(interruptedAnswerSentences, index);
    if (!text) {
        setStatus("Выберите предложение, с которого нужно продолжить ответ.", "error");
        return;
    }
    resumeAnswerButton.disabled = true;
    let prepared = null;
    try {
        if (resumeSourceRequest === null)
            throw new Error("RESUME_SOURCE_UNAVAILABLE");
        prepared = await api("/api/avatar/resume-answer", {
            request_sequence: resumeSourceRequest,
            sentence_index: index,
        });
        await syncStatus();
        if (sessionState.backend.session_state !== "active") {
            throw new Error("INVALID_STATE_TRANSITION");
        }
        // Server-owned Echo already delivered and marked Sent in the Rust turn.
        // Never send a browser did.speak or duplicate its delivery ACK.
        if (prepared.client_command !== null) {
            await dispatchClientCommand(prepared.client_command);
            await api("/api/avatar/client-delivery-sent", {
                evidence_turn_sequence: prepared.evidence_turn_sequence,
                evidence_output_sequence: prepared.evidence_output_sequence,
            });
        }
        resumeFromIndex = index;
        resumedSpeechStartedAt = performance.now();
        providerPlaybackInFlight = true;
        resumeAnswerRow.hidden = true;
        setStatus("Продолжаю сохранённый ответ с выбранного предложения (без нового LLM-запроса). "
            + "Это повторная отправка текста, а не подтверждение полного playback.", "ready");
    }
    catch (error) {
        if (prepared !== null) {
            closePeerTransport();
            let cleanupFailed = false;
            try {
                await api("/api/session/revoke", sessionEndRequest());
            }
            catch {
                cleanupFailed = true;
            }
            await syncStatus().catch(() => undefined);
            const canonicalRevoked = ["revoked", "closed"].includes(sessionState.backend.session_state);
            const cause = error instanceof Error ? error.message : "RESUME_DELIVERY_UNCONFIRMED";
            setStatus(!canonicalRevoked
                ? `RESUME_DELIVERY_UNCONFIRMED_REVOKE_FAILED: ${cause}`
                : cleanupFailed
                    ? `RESUME_DELIVERY_UNCONFIRMED_CANONICAL_REVOKED_CLEANUP_PENDING: ${cause}`
                    : `RESUME_DELIVERY_UNCONFIRMED_SESSION_REVOKED: ${cause}`, "error");
            updateControls();
            return;
        }
        setStatus(error instanceof Error ? error.message : "Не удалось продолжить ответ", "error");
    }
    updateControls();
};
const endSession = async (kind) => {
    if (!sessionEndRequest().expected_session_sequence) {
        closePeerTransport();
        setStatus("SESSION_GENERATION_UNKNOWN: завершение отклонено для защиты другой сессии", "error");
        return;
    }
    broadcastSessionEgressFence();
    closePeerTransport();
    let fenceError = null;
    if (kind === "close") {
        try {
            await api("/api/session/fence", sessionEndRequest());
        }
        catch (error) {
            fenceError = error instanceof Error ? error : new Error("CANONICAL_FENCE_FAILED");
        }
    }
    const connectionEvidenceError = kind === "revoke"
        ? null : fenceError ?? await tryFlushConnectionMediaEvidence();
    if (kind === "close" && !fenceError && rt0EvidenceMode && pendingAvSyncEvidence) {
        setStatus("RT0 evidence: завершаю ограниченный сбор A/V-sync перед закрытием…", "idle");
        await pendingAvSyncEvidence.catch(() => undefined);
    }
    try {
        await api(`/api/session/${kind}`, sessionEndRequest());
        await syncStatus();
        await refreshSessionEvidence();
        if (kind === "revoke") {
            setStatus(connectionEvidenceError
                ? `Доступ отозван. Connection evidence incomplete: ${connectionEvidenceError.message}`
                : "Доступ отозван. Сессию можно закрыть.", connectionEvidenceError ? "error" : "idle");
        }
        else {
            try {
                await downloadSessionEvidence();
                setStatus(connectionEvidenceError
                    ? `Сессия закрыта. Evidence snapshot сохранён. Connection evidence incomplete: ${connectionEvidenceError.message}`
                    : "Сессия закрыта. Evidence snapshot сохранён.", connectionEvidenceError ? "error" : "idle");
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
    if (!sessionEndRequest().expected_session_sequence)
        return;
    broadcastSessionEgressFence();
    closePeerTransport();
    void runtimeFetch("/api/session/close", {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-VPR-CSRF": csrfToken },
        body: JSON.stringify(sessionEndRequest()),
        credentials: "same-origin",
        cache: "no-store",
        keepalive: true,
    }).catch(() => undefined);
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
interruptButton.addEventListener("click", () => void interruptAndOfferResume());
resumeAnswerButton.addEventListener("click", () => void resumeInterruptedAnswer());
revokeButton.addEventListener("click", () => void endSession("revoke"));
closeButton.addEventListener("click", () => void endSession("close"));
voiceButton.addEventListener("click", () => void toggleVoice());
microphoneSelect.addEventListener("change", () => {
    rememberMicrophoneDeviceId(microphoneSelect.value.trim());
});
window.addEventListener("pagehide", closeBackendOnUnload);
void api("/api/bootstrap")
    .then(async (bootstrap) => {
    csrfToken = bootstrap.csrf_token;
    publishBootstrap(csrfToken);
    egressEnabled = bootstrap.egress_enabled;
    rt0EvidenceMode = bootstrap.rt0_evidence_mode === true;
    await syncStatus();
    ownerCaptureReviewed = sessionState.backend.owner_context_state === "reviewed";
    if (sessionState.backend.session_audience)
        audienceSelect.value = sessionState.backend.session_audience;
    if (sessionState.backend.session_audience !== "visitor") {
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
    bootstrapComplete = true;
})
    .catch((error) => setStatus(error instanceof Error ? error.message : "Ошибка bootstrap", "error"));
