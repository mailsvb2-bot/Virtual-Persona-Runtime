import { mountOwnerCapture } from "./owner-capture.js";

type Bootstrap = { csrf_token: string; egress_enabled: boolean };
type SessionAudience = "owner" | "visitor";
type ConversationReadiness = "none" | "text" | "text_and_voice";
type LabStatus = { session_state: string; avatar_open: boolean; egress_enabled: boolean; conversation_readiness: ConversationReadiness; session_audience: SessionAudience | null; owner_context_state: "missing" | "reviewed"; persona_version: number; reviewed_owner_claims: number };
type TextResult = { reply: string; locale: string; evidence_turn_sequence: number; first_meaningful_response_millis: number; total_millis: number };
type VoiceResult = { transcript: string; reply: string; locale: string; evidence_turn_sequence: number; evidence_output_sequence: number; stt_millis: number; llm_millis: number; avatar_millis: number; total_millis: number };
type SessionDescription = { kind: RTCSdpType; sdp: string };
type IceServer = { urls: string[]; username: string | null; credential: string | null };
type ClientControl = { data_channel_label: string; interrupt: boolean };
type ClientCommand = { data_channel_label: string; payload: string };
type ClientEvent =
  | { kind: "playback_started"; playback_id: string }
  | { kind: "playback_done" };
type StartResponse = { evidence_session_sequence: number; offer: SessionDescription; ice_servers: IceServer[]; capabilities: string[]; client_control: ClientControl | null };
type ErrorPayload = { ok: false; code: string };
type IceCandidatePayload = { candidate: string | null; sdpMid: string | null; sdpMLineIndex: number | null };
type MediaEvidenceKind = "video_ready" | "audio_started" | "interruption_stopped" | "reconnect_restored";
type AvSyncReference = "web_rtc_estimated_playout_timestamp";
type InboundRtpSyncStat = { type?: string; kind?: string; mediaType?: string; estimatedPlayoutTimestamp?: number; packetsReceived?: number };
type ActiveVoiceEvidence = { requestSequence: number; startedAt: number; audioStarted: boolean; audioStartedElapsed: number | null; responseComplete: boolean; speaking: boolean; silentFrames: number };
type InterruptEvidenceWatch = { requestSequence: number; startedAt: number; silentFrames: number };

const byId = <T extends HTMLElement>(id: string): T => {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing element ${id}`);
  return element as T;
};

const video = byId<HTMLVideoElement>("avatar");
const stage = document.querySelector<HTMLElement>(".stage");
const personaPanel = byId<HTMLElement>("persona-panel");
const audienceSelect = byId<HTMLSelectElement>("session-audience");
const visitorOption = audienceSelect.querySelector<HTMLOptionElement>('option[value="visitor"]');
const consent = byId<HTMLInputElement>("consent");
const message = byId<HTMLTextAreaElement>("message");
const connectButton = byId<HTMLButtonElement>("connect");
const speakButton = byId<HTMLButtonElement>("speak");
const interruptButton = byId<HTMLButtonElement>("interrupt");
const revokeButton = byId<HTMLButtonElement>("revoke");
const closeButton = byId<HTMLButtonElement>("close");
const voiceButton = byId<HTMLButtonElement>("voice");
const statusNode = byId<HTMLElement>("status");
const evidenceNode = byId<HTMLElement>("evidence");

let csrfToken = "";
let egressEnabled = false;
let backendStatus: LabStatus = { session_state: "none", avatar_open: false, egress_enabled: false, conversation_readiness: "none", session_audience: null, owner_context_state: "missing", persona_version: 1, reviewed_owner_claims: 0 };
let ownerCaptureReviewed = false;
let peer: RTCPeerConnection | null = null;
let providerDataChannel: RTCDataChannel | null = null;
let providerPlaybackId: string | null = null;
let answerSubmitted = false;
let pendingIce: IceCandidatePayload[] = [];
let capabilities = new Set<string>();
let micStream: MediaStream | null = null;
let audioContext: AudioContext | null = null;
let micSource: MediaStreamAudioSourceNode | null = null;
let micWorklet: AudioWorkletNode | null = null;
let micChunks: Float32Array[] = [];
let recording = false;
let recordingTimer: number | null = null;
let textRequestInFlight = false;
let voiceRequestInFlight = false;
let evidenceSessionSequence = 0;
let nextTextRequestSequence = 0;
let nextVoiceRequestSequence = 0;
let connectEvidenceStartedAt = 0;
let videoEvidencePosted = false;
let reconnectStartedAt: number | null = null;
let remoteMediaStream: MediaStream | null = null;
let remoteEvidenceAudioContext: AudioContext | null = null;
let remoteAudioSource: MediaStreamAudioSourceNode | null = null;
let remoteAudioAnalyser: AnalyserNode | null = null;
let remoteSilentGain: GainNode | null = null;
let remoteEvidenceFrame: number | null = null;
let baselineRms = 0.002;
let activeVoiceEvidence: ActiveVoiceEvidence | null = null;
let interruptEvidenceWatch: InterruptEvidenceWatch | null = null;
const MAX_VOICE_SAMPLES = 480_000;
const AUTO_STOP_MILLIS = 29_500;
const AV_SYNC_REFERENCE: AvSyncReference = "web_rtc_estimated_playout_timestamp";
const AV_SYNC_SAMPLE_COUNT = 3;
const AV_SYNC_SAMPLE_INTERVAL_MILLIS = 100;
const AV_SYNC_MAX_ATTEMPTS = 12;

const setStatus = (text: string, state: "idle" | "ready" | "error" = "idle"): void => {
  statusNode.textContent = text;
  statusNode.dataset.state = state;
};

const showEvidence = (value: unknown): void => {
  evidenceNode.textContent = JSON.stringify(value, null, 2);
};

const api = async <T>(path: string, body?: unknown): Promise<T> => {
  const init: RequestInit = body === undefined
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
  const payload = await response.json() as T | ErrorPayload;
  if (!response.ok) {
    const code = (payload as ErrorPayload).code ?? `HTTP_${response.status}`;
    throw new Error(code);
  }
  return payload as T;
};

const apiEvidenceJson = async <T>(path: string, body: unknown, requestSequence: number): Promise<T> => {
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
  const payload = await response.json() as T | ErrorPayload;
  if (!response.ok) {
    const code = (payload as ErrorPayload).code ?? `HTTP_${response.status}`;
    throw new Error(code);
  }
  return payload as T;
};

const apiBinary = async <T>(path: string, body: ArrayBuffer, requestSequence: number): Promise<T> => {
  const response = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/octet-stream", "X-VPR-CSRF": csrfToken, "X-VPR-Evidence-Request": String(requestSequence) },
    body,
    credentials: "same-origin",
    cache: "no-store",
  });
  const payload = await response.json() as T | ErrorPayload;
  if (!response.ok) {
    const code = (payload as ErrorPayload).code ?? `HTTP_${response.status}`;
    throw new Error(code);
  }
  return payload as T;
};

const refreshSessionEvidence = async (): Promise<void> => {
  if (evidenceSessionSequence === 0) return;
  try {
    showEvidence(await api<unknown>("/api/evidence/session"));
  } catch {
    // Evidence visibility is best-effort UI; the backend recorder itself remains fail-closed.
  }
};

const postMediaEvidence = async (
  kind: MediaEvidenceKind,
  elapsedMillis: number,
  requestSequence: number | null = null,
): Promise<void> => {
  if (evidenceSessionSequence === 0) return;
  await api<{ ok: true }>("/api/evidence/media", {
    session_sequence: evidenceSessionSequence,
    request_sequence: requestSequence,
    kind,
    elapsed_millis: Math.max(0, Math.round(elapsedMillis)),
  });
  await refreshSessionEvidence();
};

const readAvSyncOffsetMillis = async (): Promise<number | null> => {
  const currentPeer = peer;
  if (!currentPeer) return null;
  const audio: number[] = [];
  const videoOffsets: number[] = [];
  const stats = await currentPeer.getStats();
  stats.forEach((raw) => {
    const stat = raw as unknown as InboundRtpSyncStat;
    if (stat.type !== "inbound-rtp" || !Number.isFinite(stat.estimatedPlayoutTimestamp)) return;
    const packetsReceived = stat.packetsReceived;
    if (packetsReceived === undefined || !Number.isFinite(packetsReceived) || packetsReceived <= 0) return;
    const kind = stat.kind ?? stat.mediaType;
    if (kind === "audio") audio.push(stat.estimatedPlayoutTimestamp as number);
    else if (kind === "video") videoOffsets.push(stat.estimatedPlayoutTimestamp as number);
  });
  if (audio.length !== 1 || videoOffsets.length !== 1) return null;
  const audioTimestamp = audio[0];
  const videoTimestamp = videoOffsets[0];
  if (audioTimestamp === undefined || videoTimestamp === undefined) return null;
  return Math.round(Math.abs(audioTimestamp - videoTimestamp));
};

const collectAvSyncEvidence = async (requestSequence: number): Promise<void> => {
  let sampleSequence = 1;
  let attempts = 0;
  while (sampleSequence <= AV_SYNC_SAMPLE_COUNT && attempts < AV_SYNC_MAX_ATTEMPTS) {
    attempts += 1;
    const absoluteOffsetMillis = await readAvSyncOffsetMillis();
    if (absoluteOffsetMillis !== null) {
      await api<{ ok: true }>("/api/evidence/av-sync", {
        session_sequence: evidenceSessionSequence,
        request_sequence: requestSequence,
        sample_sequence: sampleSequence,
        reference: AV_SYNC_REFERENCE,
        absolute_offset_millis: absoluteOffsetMillis,
      });
      sampleSequence += 1;
    }
    if (sampleSequence <= AV_SYNC_SAMPLE_COUNT && attempts < AV_SYNC_MAX_ATTEMPTS) {
      await new Promise<void>((resolve) => window.setTimeout(resolve, AV_SYNC_SAMPLE_INTERVAL_MILLIS));
    }
  }
  if (sampleSequence > 1) await refreshSessionEvidence();
};

const rms = (samples: Float32Array): number => {
  let sum = 0;
  for (const sample of samples) sum += sample * sample;
  return Math.sqrt(sum / Math.max(1, samples.length));
};

const stopRemoteEvidence = (): void => {
  if (remoteEvidenceFrame !== null) cancelAnimationFrame(remoteEvidenceFrame);
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

const monitorRemoteAudio = (): void => {
  const analyser = remoteAudioAnalyser;
  if (!analyser) return;
  const samples = new Float32Array(analyser.fftSize);
  const tick = (): void => {
    const current = remoteAudioAnalyser;
    if (!current) return;
    current.getFloatTimeDomainData(samples);
    const level = rms(samples);
    if (!activeVoiceEvidence) baselineRms = baselineRms * 0.95 + level * 0.05;
    const voice = activeVoiceEvidence;
    const speechThreshold = Math.max(0.015, baselineRms * 3 + 0.003);
    if (voice && level > speechThreshold) {
      voice.speaking = true;
      voice.silentFrames = 0;
      if (!voice.audioStarted) {
        voice.audioStarted = true;
        voice.audioStartedElapsed = performance.now() - voice.startedAt;
        if (voice.responseComplete) {
          void postMediaEvidence("audio_started", voice.audioStartedElapsed, voice.requestSequence)
            .then(() => collectAvSyncEvidence(voice.requestSequence))
            .catch(() => undefined);
        }
      }
    } else if (voice?.speaking) {
      voice.silentFrames += 1;
      if (voice.silentFrames >= 6) voice.speaking = false;
    }
    if (interruptEvidenceWatch) {
      const silenceThreshold = Math.max(0.008, baselineRms * 1.8 + 0.002);
      interruptEvidenceWatch.silentFrames = level < silenceThreshold
        ? interruptEvidenceWatch.silentFrames + 1
        : 0;
      if (interruptEvidenceWatch.silentFrames >= 4) {
        const watch = interruptEvidenceWatch;
        interruptEvidenceWatch = null;
        void postMediaEvidence(
          "interruption_stopped",
          performance.now() - watch.startedAt,
          watch.requestSequence,
        ).catch(() => undefined);
      }
    }
    remoteEvidenceFrame = requestAnimationFrame(tick);
  };
  remoteEvidenceFrame = requestAnimationFrame(tick);
};

const attachRemoteAudioEvidence = async (track: MediaStreamTrack): Promise<void> => {
  if (!remoteEvidenceAudioContext) remoteEvidenceAudioContext = new AudioContext();
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
  if (remoteEvidenceFrame === null) monitorRemoteAudio();
};

const recordFirstVideoFrame = (): void => {
  if (videoEvidencePosted || connectEvidenceStartedAt === 0) return;
  videoEvidencePosted = true;
  void postMediaEvidence("video_ready", performance.now() - connectEvidenceStartedAt)
    .catch(() => undefined);
};

const syncStatus = async (): Promise<LabStatus> => {
  backendStatus = await api<LabStatus>("/api/status");
  updateControls();
  showEvidence(backendStatus);
  return backendStatus;
};

const postIce = async (candidate: IceCandidatePayload): Promise<void> => {
  await api<{ ok: true }>("/api/avatar/ice", {
    candidate: candidate.candidate ?? null,
    sdp_mid: candidate.sdpMid ?? null,
    sdp_mline_index: candidate.sdpMLineIndex ?? null,
  });
};

const flushIce = async (): Promise<void> => {
  const queued = pendingIce;
  pendingIce = [];
  for (const candidate of queued) await postIce(candidate);
};

const backendSessionPresent = (): boolean => !["none", "closed"].includes(backendStatus.session_state);

const selectedAudience = (): SessionAudience => backendStatus.session_audience ?? (audienceSelect.value as SessionAudience);

const updateAudienceMode = (): void => {
  personaPanel.hidden = selectedAudience() === "visitor";
};

const updateControls = (): void => {
  const transportReady = peer !== null && answerSubmitted && backendStatus.session_state === "active";
  const textReady = backendStatus.conversation_readiness !== "none";
  const voiceReady = backendStatus.conversation_readiness === "text_and_voice";
  const clientInterruptReady = providerDataChannel?.readyState === "open" && providerPlaybackId !== null;
  speakButton.disabled = !transportReady || !textReady || textRequestInFlight || voiceRequestInFlight;
  interruptButton.disabled = !textRequestInFlight && !voiceRequestInFlight && !clientInterruptReady;
  voiceButton.disabled = recording ? false : !transportReady || !voiceReady || textRequestInFlight || voiceRequestInFlight;
  voiceButton.textContent = recording ? "Остановить и отправить" : "Начать говорить";
  revokeButton.disabled = !backendSessionPresent()
    || (backendStatus.session_state === "revoked" && !backendStatus.avatar_open);
  closeButton.disabled = !backendSessionPresent();
  connectButton.disabled = !egressEnabled || backendSessionPresent() || !ownerCaptureReviewed;
  audienceSelect.disabled = backendSessionPresent();
  if (visitorOption) visitorOption.disabled = !ownerCaptureReviewed;
  updateAudienceMode();
};

const ownerCapture = mountOwnerCapture({
  api,
  onStateChange: (state) => {
    ownerCaptureReviewed = state.reviewed;
    updateControls();
  },
});

const closePeerTransport = (): void => {
  stopMicrophoneCapture();
  stopRemoteEvidence();
  providerDataChannel?.close();
  providerDataChannel = null;
  providerPlaybackId = null;
  peer?.close();
  peer = null;
  video.srcObject = null;
  stage?.classList.remove("has-video");
  answerSubmitted = false;
  pendingIce = [];
  capabilities.clear();
  updateControls();
};

const connectAvatar = async (): Promise<void> => {
  if (!consent.checked) {
    setStatus("Нужно явное согласие", "error");
    return;
  }
  connectButton.disabled = true;
  connectEvidenceStartedAt = performance.now();
  videoEvidencePosted = false;
  reconnectStartedAt = null;
  evidenceSessionSequence = 0;
  nextTextRequestSequence = 0;
  nextVoiceRequestSequence = 0;
  remoteEvidenceAudioContext = new AudioContext();
  void remoteEvidenceAudioContext.resume();
  setStatus("Создаю защищённую сессию…");
  try {
    const audience = audienceSelect.value as SessionAudience;
    const start = await api<StartResponse>("/api/avatar/start", { consent: true, audience });
    evidenceSessionSequence = start.evidence_session_sequence;
    backendStatus = { ...backendStatus, session_state: "active", avatar_open: true, egress_enabled: egressEnabled, session_audience: audience };
    capabilities = new Set(start.capabilities);
    updateControls();
    peer = new RTCPeerConnection({
      iceServers: start.ice_servers.map((server) => ({
        urls: server.urls,
        ...(server.username ? { username: server.username } : {}),
        ...(server.credential ? { credential: server.credential } : {}),
      })),
    });
    if (start.client_control?.interrupt) {
      const channel = peer.createDataChannel(start.client_control.data_channel_label);
      providerDataChannel = channel;
      channel.onopen = () => updateControls();
      channel.onclose = () => {
        providerPlaybackId = null;
        updateControls();
      };
      channel.onmessage = (event) => {
        const raw = typeof event.data === "string" ? event.data : "";
        if (!raw) return;
        void api<ClientEvent | null>("/api/avatar/client-event", { message: raw })
          .then((normalized) => {
            if (normalized?.kind === "playback_started") {
              providerPlaybackId = normalized.playback_id;
            } else if (normalized?.kind === "playback_done") {
              providerPlaybackId = null;
            }
            updateControls();
          })
          .catch(() => undefined);
      };
    }
    peer.ontrack = (event) => {
      remoteMediaStream ??= new MediaStream();
      if (!remoteMediaStream.getTracks().some((track) => track.id === event.track.id)) {
        remoteMediaStream.addTrack(event.track);
      }
      video.srcObject = remoteMediaStream;
      if (event.track.kind === "video") {
        stage?.classList.add("has-video");
        const requestFrame = (video as unknown as {
          requestVideoFrameCallback?: (callback: () => void) => number;
        }).requestVideoFrameCallback;
        if (typeof requestFrame === "function") {
          requestFrame.call(video, () => recordFirstVideoFrame());
        } else {
          video.addEventListener("playing", recordFirstVideoFrame, { once: true });
        }
        setStatus("Видео подключено", "ready");
      } else if (event.track.kind === "audio") {
        void attachRemoteAudioEvidence(event.track).catch(() => undefined);
      }
    };
    peer.onconnectionstatechange = () => {
      if (!peer) return;
      const state = peer.connectionState;
      if ((state === "disconnected" || state === "failed") && reconnectStartedAt === null) {
        reconnectStartedAt = performance.now();
      } else if (state === "connected" && reconnectStartedAt !== null) {
        const startedAt = reconnectStartedAt;
        reconnectStartedAt = null;
        void postMediaEvidence("reconnect_restored", performance.now() - startedAt)
          .catch(() => undefined);
      }
      if (state === "failed") setStatus("WebRTC connection failed", "error");
      void refreshSessionEvidence();
    };
    peer.onicecandidate = (event) => {
      const json = event.candidate?.toJSON();
      const candidate: IceCandidatePayload = {
        candidate: json?.candidate ?? null,
        sdpMid: json?.sdpMid ?? null,
        sdpMLineIndex: json?.sdpMLineIndex ?? null,
      };
      if (!answerSubmitted) pendingIce.push(candidate);
      else void postIce(candidate).catch((error: unknown) => setStatus(String(error), "error"));
    };

    await peer.setRemoteDescription({ type: start.offer.kind, sdp: start.offer.sdp });
    const answer = await peer.createAnswer();
    await peer.setLocalDescription(answer);
    await api<{ ok: true }>("/api/avatar/answer", { kind: answer.type, sdp: answer.sdp ?? "" });
    answerSubmitted = true;
    await flushIce();

    updateControls();
    setStatus(selectedAudience() === "visitor" ? "Visitor-сессия WebRTC согласована" : "WebRTC согласован", "ready");
    showEvidence({ connectionState: peer.connectionState, capabilities: [...capabilities] });
  } catch (error) {
    const messageText = error instanceof Error ? error.message : "Ошибка подключения";
    closePeerTransport();
    if (backendSessionPresent()) {
      try {
        await api<{ ok: true }>("/api/session/close", {});
        await syncStatus();
      } catch (cleanupError) {
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

const stopMicrophoneCapture = (): void => {
  if (recordingTimer !== null) window.clearTimeout(recordingTimer);
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

const flattenChunks = (chunks: Float32Array[]): Float32Array => {
  const total = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const output = new Float32Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    output.set(chunk, offset);
    offset += chunk.length;
  }
  return output;
};

const resampleMono = (input: Float32Array, inputRate: number, outputRate = 16000): Float32Array => {
  if (inputRate === outputRate) return input;
  const outputLength = Math.max(1, Math.floor(input.length * outputRate / inputRate));
  const output = new Float32Array(outputLength);
  const ratio = inputRate / outputRate;
  for (let i = 0; i < outputLength; i += 1) {
    const start = Math.floor(i * ratio);
    const end = Math.min(input.length, Math.max(start + 1, Math.floor((i + 1) * ratio)));
    let sum = 0;
    for (let j = start; j < end; j += 1) sum += input[j] ?? 0;
    output[i] = sum / Math.max(1, end - start);
  }
  return output;
};

const encodeS16Le = (input: Float32Array): ArrayBuffer => {
  const buffer = new ArrayBuffer(input.length * 2);
  const view = new DataView(buffer);
  input.forEach((sample, index) => {
    const clamped = Math.max(-1, Math.min(1, sample));
    const value = clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff;
    view.setInt16(index * 2, Math.round(value), true);
  });
  return buffer;
};

const startMicrophone = async (): Promise<void> => {
  micChunks = [];
  micStream = await navigator.mediaDevices.getUserMedia({
    audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true },
  });
  audioContext = new AudioContext();
  await audioContext.audioWorklet.addModule("/mic-worklet.js");
  micSource = audioContext.createMediaStreamSource(micStream);
  micWorklet = new AudioWorkletNode(audioContext, "vpr-mic-capture");
  micWorklet.port.onmessage = (event: MessageEvent<ArrayBuffer>) => {
    micChunks.push(new Float32Array(event.data));
  };
  micSource.connect(micWorklet);
  micWorklet.connect(audioContext.destination);
  recording = true;
  recordingTimer = window.setTimeout(() => void finishMicrophoneTurn(), AUTO_STOP_MILLIS);
  setStatus("Слушаю… нажмите ещё раз, чтобы отправить", "ready");
  updateControls();
};

const finishMicrophoneTurn = async (): Promise<void> => {
  if (!recording || !audioContext) return;
  const inputRate = audioContext.sampleRate;
  const samples = flattenChunks(micChunks);
  stopMicrophoneCapture();
  micChunks = [];
  if (samples.length === 0) {
    setStatus("Микрофон не записал звук", "error");
    return;
  }
  voiceRequestInFlight = true;
  let attemptedRequestSequence: number | null = null;
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
    activeVoiceEvidence = {
      requestSequence,
      startedAt: performance.now(),
      audioStarted: false,
      audioStartedElapsed: null,
      responseComplete: false,
      speaking: false,
      silentFrames: 0,
    };
    const result = await apiBinary<VoiceResult>("/api/voice/turn", pcm, requestSequence);
    const voice = activeVoiceEvidence;
    if (voice?.requestSequence === requestSequence) {
      voice.responseComplete = true;
      if (voice.audioStartedElapsed !== null) {
        await postMediaEvidence("audio_started", voice.audioStartedElapsed, requestSequence);
        await collectAvSyncEvidence(requestSequence).catch(() => undefined);
      }
    }
    await refreshSessionEvidence();
    setStatus(`Вы: ${result.transcript} · Ответ: ${result.reply}`, "ready");
  } catch (error) {
    if (activeVoiceEvidence?.requestSequence === attemptedRequestSequence) activeVoiceEvidence = null;
    await refreshSessionEvidence();
    setStatus(error instanceof Error ? error.message : "Ошибка голосового запроса", "error");
  } finally {
    voiceRequestInFlight = false;
    updateControls();
  }
};

const toggleVoice = async (): Promise<void> => {
  try {
    if (recording) await finishMicrophoneTurn();
    else await startMicrophone();
  } catch (error) {
    stopMicrophoneCapture();
    setStatus(error instanceof Error ? error.message : "Ошибка микрофона", "error");
  }
};

const speak = async (): Promise<void> => {
  const text = message.value.trim();
  if (!text) return;
  const requestSequence = ++nextTextRequestSequence;
  textRequestInFlight = true;
  updateControls();
  try {
    const result = await apiEvidenceJson<TextResult>("/api/text/turn", { text }, requestSequence);
    setStatus(`Ответ: ${result.reply}`, "ready");
    message.value = "";
    await refreshSessionEvidence();
  } catch (error) {
    setStatus(error instanceof Error ? error.message : "Ошибка текстового разговора", "error");
  } finally {
    textRequestInFlight = false;
    updateControls();
  }
};

const interruptAvatar = async (): Promise<void> => {
  const voice = activeVoiceEvidence;
  if (voice?.audioStarted) {
    interruptEvidenceWatch = {
      requestSequence: voice.requestSequence,
      startedAt: performance.now(),
      silentFrames: 0,
    };
  }

  const channel = providerDataChannel;
  const playbackId = providerPlaybackId;
  const clientReady = !textRequestInFlight
    && !voiceRequestInFlight
    && channel?.readyState === "open"
    && playbackId !== null;
  try {
    if (clientReady && channel && playbackId) {
      const command = await api<ClientCommand>("/api/avatar/client-interrupt", {
        playback_id: playbackId,
      });
      if (channel.readyState !== "open" || channel.label !== command.data_channel_label) {
        throw new Error("CLIENT_INTERRUPT_CHANNEL_MISMATCH");
      }
      channel.send(command.payload);
      providerPlaybackId = null;
      updateControls();
      await refreshSessionEvidence();
      return;
    }
    await api<{ ok: true }>("/api/avatar/interrupt", {});
    await refreshSessionEvidence();
  } catch (error) {
    interruptEvidenceWatch = null;
    setStatus(error instanceof Error ? error.message : "Ошибка прерывания", "error");
    updateControls();
  }
};

const endSession = async (kind: "revoke" | "close"): Promise<void> => {
  closePeerTransport();
  try {
    await api<{ ok: true }>(`/api/session/${kind}`, {});
    await syncStatus();
    await refreshSessionEvidence();
    setStatus(
      kind === "revoke" ? "Доступ отозван. Сессию можно закрыть." : "Сессия закрыта",
      "idle",
    );
  } catch (error) {
    await syncStatus().catch(() => undefined);
    setStatus(error instanceof Error ? `${error.message}; повторите завершение` : "Ошибка завершения", "error");
  } finally {
    updateControls();
  }
};

const closeBackendOnUnload = (): void => {
  if (!backendSessionPresent() || !csrfToken) return;
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
  } else {
    setStatus(
      audienceSelect.value === "visitor"
        ? "Visitor preview: owner-reviewed личный контекст закрыт"
        : "Owner preview: reviewed owner context доступен каноническому runtime",
      "ready",
    );
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

void api<Bootstrap>("/api/bootstrap")
  .then(async (bootstrap) => {
    csrfToken = bootstrap.csrf_token;
    egressEnabled = bootstrap.egress_enabled;
    await syncStatus();
    ownerCaptureReviewed = backendStatus.owner_context_state === "reviewed";
    if (backendStatus.session_audience) audienceSelect.value = backendStatus.session_audience;
    if (backendStatus.session_audience !== "visitor") {
      await ownerCapture.refresh();
    } else {
      personaPanel.hidden = true;
    }
    if (!egressEnabled) {
      setStatus("Egress выключен на backend", "error");
    } else if (backendSessionPresent()) {
      setStatus("Найдена незакрытая сессия — доступно безопасное завершение", "error");
    } else if (!ownerCaptureReviewed) {
      setStatus("Сначала создайте и подтвердите Persona");
    } else {
      setStatus("Persona подтверждена. Готов к подключению", "ready");
    }
    updateControls();
  })
  .catch((error: unknown) => setStatus(error instanceof Error ? error.message : "Ошибка bootstrap", "error"));
