import { mountOwnerCapture } from "./owner-capture.js";
import { PlaybackAwareCommandScheduler } from "./voice-command-scheduler.js";

type Bootstrap = { csrf_token: string; egress_enabled: boolean };
type SessionAudience = "owner" | "visitor";
type ConversationReadiness = "none" | "text" | "text_and_voice";
type LabStatus = { session_state: string; avatar_open: boolean; egress_enabled: boolean; conversation_readiness: ConversationReadiness; session_audience: SessionAudience | null; owner_context_state: "missing" | "reviewed"; persona_version: number; reviewed_owner_claims: number };
type TextResult = { reply: string; locale: string; evidence_turn_sequence: number; first_meaningful_response_millis: number; total_millis: number };
type ClientRoute =
  | { kind: "web_rtc_data_channel"; label: string }
  | { kind: "live_kit_text_topic"; topic: string };
type ClientCommand = { route: ClientRoute; payload: string };
type VoiceResult = { transcript: string; reply: string; locale: string; evidence_turn_sequence: number; evidence_output_sequence: number; stt_millis: number; llm_millis: number; llm_first_meaningful_millis: number; avatar_millis: number; total_millis: number; client_command: ClientCommand | null };
type VoiceStartAck = { ok: true; request_sequence: number };
type VoiceSegment = { evidence_turn_sequence: number; evidence_output_sequence: number; client_command: ClientCommand | null };
type VoiceStreamEvent =
  | { kind: "segment"; segment: VoiceSegment }
  | { kind: "complete"; result: VoiceResult }
  | { kind: "failed"; code: string };
type VoiceEventsResponse = { events: VoiceStreamEvent[]; terminal: boolean };
type SessionDescription = { kind: RTCSdpType; sdp: string };
type IceServer = { urls: string[]; username: string | null; credential: string | null };
type RealtimeTransport =
  | { kind: "web_rtc"; offer: SessionDescription; ice_servers: IceServer[] }
  | { kind: "live_kit"; server_url: string; token: string };
type ClientControl = {
  event_route: ClientRoute | null;
  interrupt: boolean;
  interrupt_requires_playback_id: boolean;
  text_input: boolean;
};
type ClientEvent =
  | { kind: "playback_started"; playback_id: string }
  | { kind: "playback_done" };
type StartResponse = { evidence_session_sequence: number; transport: RealtimeTransport; capabilities: string[]; client_control: ClientControl | null };
type ErrorPayload = { ok: false; code: string };
type IceCandidatePayload = { candidate: string | null; sdpMid: string | null; sdpMLineIndex: number | null };
type MediaEvidenceKind = "video_ready" | "audio_started" | "interruption_stopped" | "reconnect_restored";
type AvSyncReference = "web_rtc_estimated_playout_timestamp";
type InboundRtpSyncStat = { type?: string; kind?: string; mediaType?: string; estimatedPlayoutTimestamp?: number; packetsReceived?: number };
type ActiveVoiceEvidence = { requestSequence: number; startedAt: number; audioStarted: boolean; audioStartedElapsed: number | null; responseComplete: boolean; speaking: boolean; silentFrames: number };
type InterruptEvidenceWatch = { requestSequence: number; startedAt: number; silentFrames: number };

type UsageEvidence = {
  input_units: number | null;
  output_units: number | null;
  estimated_cost_microunits: number | null;
  provider_charge_microunits: number | null;
};
type TextAttemptEvidence = {
  request_sequence: number;
  status: string;
  first_meaningful_response_millis: number | null;
  server_total_millis: number | null;
  llm_usage: UsageEvidence | null;
};
type VoiceAttemptEvidence = {
  request_sequence: number;
  canonical_playback_confirmed: boolean;
  status: string;
  stt_millis: number | null;
  llm_millis: number | null;
  llm_first_meaningful_millis: number | null;
  avatar_millis: number | null;
  server_total_millis: number | null;
  stt_usage: UsageEvidence | null;
  llm_usage: UsageEvidence | null;
};
type MediaEventEvidence = {
  request_sequence: number | null;
  kind: MediaEvidenceKind;
  elapsed_millis: number;
};
type AvSyncEvidence = {
  request_sequence: number;
  sample_sequence: number;
  absolute_offset_millis: number;
};
type SessionEvidenceSnapshot = {
  schema_version: string;
  canonical_playback_proven: boolean;
  av_sync_proven: boolean;
  text_attempts: TextAttemptEvidence[];
  voice_attempts: VoiceAttemptEvidence[];
  media_events: MediaEventEvidence[];
  av_sync_samples: AvSyncEvidence[];
};


type LiveKitTrack = {
  kind: string;
  mediaStreamTrack?: MediaStreamTrack;
  attach: (element: HTMLMediaElement) => HTMLMediaElement;
  detach?: (element?: HTMLMediaElement) => HTMLMediaElement[];
  getRTCStatsReport?: () => Promise<RTCStatsReport | undefined>;
};
type LiveKitParticipant = {
  sendText: (text: string, options: { topic: string }) => Promise<void>;
};
type LiveKitRoom = {
  localParticipant: LiveKitParticipant;
  connect: (url: string, token: string) => Promise<void>;
  disconnect: () => Promise<void>;
  on: (event: string, listener: (...args: unknown[]) => void) => LiveKitRoom;
};
type LiveKitSdk = {
  Room: new () => LiveKitRoom;
  RoomEvent: {
    TrackSubscribed: string;
    TrackUnsubscribed: string;
    DataReceived: string;
    Reconnecting: string;
    Reconnected: string;
    Disconnected: string;
  };
};

const LIVEKIT_CLIENT_URL =
  "https://cdn.jsdelivr.net/npm/livekit-client@2.22.3/dist/livekit-client.umd.min.js";
let liveKitLoader: Promise<LiveKitSdk> | null = null;

const loadLiveKitSdk = async (): Promise<LiveKitSdk> => {
  const existing = (window as typeof window & { LivekitClient?: LiveKitSdk }).LivekitClient;
  if (existing) return existing;
  liveKitLoader ??= new Promise<LiveKitSdk>((resolve, reject) => {
    const script = document.createElement("script");
    script.src = LIVEKIT_CLIENT_URL;
    script.async = true;
    script.crossOrigin = "anonymous";
    script.onload = () => {
      const sdk = (window as typeof window & { LivekitClient?: LiveKitSdk }).LivekitClient;
      if (sdk) resolve(sdk);
      else reject(new Error("LIVEKIT_CLIENT_UNAVAILABLE"));
    };
    script.onerror = () => reject(new Error("LIVEKIT_CLIENT_LOAD_FAILED"));
    document.head.append(script);
  });
  return liveKitLoader;
};

const byId = <T extends HTMLElement>(id: string): T => {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing element ${id}`);
  return element as T;
};

const video = byId<HTMLVideoElement>("avatar");
const avatarAudio = byId<HTMLAudioElement>("avatar-audio");
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
const metricStt = byId<HTMLElement>("metric-stt");
const metricLlm = byId<HTMLElement>("metric-llm");
const metricLlmFirst = byId<HTMLElement>("metric-llm-first");
const metricServerTotal = byId<HTMLElement>("metric-server-total");
const metricTextFirst = byId<HTMLElement>("metric-text-first");
const metricFirstAudio = byId<HTMLElement>("metric-first-audio");
const metricVideoReady = byId<HTMLElement>("metric-video-ready");
const metricAvSync = byId<HTMLElement>("metric-av-sync");
const metricPlayback = byId<HTMLElement>("metric-playback");
const metricCost = byId<HTMLElement>("metric-cost");

let csrfToken = "";
let egressEnabled = false;
let backendStatus: LabStatus = { session_state: "none", avatar_open: false, egress_enabled: false, conversation_readiness: "none", session_audience: null, owner_context_state: "missing", persona_version: 1, reviewed_owner_claims: 0 };
let ownerCaptureReviewed = false;
let peer: RTCPeerConnection | null = null;
let liveKitRoom: LiveKitRoom | null = null;
let liveKitAudioTrack: LiveKitTrack | null = null;
let liveKitVideoTrack: LiveKitTrack | null = null;
let realtimeReadiness = { control: false, audio: false, video: false };
let providerDataChannel: RTCDataChannel | null = null;
let activeClientControl: ClientControl | null = null;
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

const formatMillis = (value: number | null | undefined): string =>
  value === null || value === undefined ? "—" : `${Math.round(value)} мс`;

const isSessionEvidenceSnapshot = (value: unknown): value is SessionEvidenceSnapshot => {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<SessionEvidenceSnapshot>;
  return typeof candidate.schema_version === "string"
    && candidate.schema_version.startsWith("rt0-owner-lab-session-evidence-")
    && Array.isArray(candidate.text_attempts)
    && Array.isArray(candidate.voice_attempts)
    && Array.isArray(candidate.media_events)
    && Array.isArray(candidate.av_sync_samples);
};

const lastCompleted = <T extends { status: string }>(items: T[]): T | undefined =>
  [...items].reverse().find((item) => item.status === "completed");

const lastMediaEvent = (
  events: MediaEventEvidence[],
  kind: MediaEvidenceKind,
  requestSequence?: number,
): MediaEventEvidence | undefined =>
  [...events].reverse().find((event) =>
    event.kind === kind
      && (requestSequence === undefined || event.request_sequence === requestSequence)
  );

const sumKnownCost = (
  usages: Array<UsageEvidence | null>,
  key: "estimated_cost_microunits" | "provider_charge_microunits",
): number | null => {
  const values = usages
    .map((usage) => usage?.[key] ?? null)
    .filter((value): value is number => value !== null && Number.isFinite(value));
  return values.length === 0 ? null : values.reduce((sum, value) => sum + value, 0);
};

const resetTelemetry = (): void => {
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

const renderTelemetry = (snapshot: SessionEvidenceSnapshot): void => {
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
  metricVideoReady.textContent = formatMillis(
    lastMediaEvent(snapshot.media_events, "video_ready")?.elapsed_millis,
  );

  const avSamples = voice
    ? snapshot.av_sync_samples.filter((sample) => sample.request_sequence === voice.request_sequence)
    : snapshot.av_sync_samples;
  if (snapshot.av_sync_proven && avSamples.length > 0) {
    const maxOffset = Math.max(...avSamples.map((sample) => sample.absolute_offset_millis));
    metricAvSync.textContent = `${maxOffset} мс · ${avSamples.length} изм.`;
  } else {
    metricAvSync.textContent = voice ? "ещё не доказан" : "—";
  }
  metricPlayback.textContent = snapshot.canonical_playback_proven
    ? "подтверждён"
    : voice ? "ожидание" : "—";

  const usages: Array<UsageEvidence | null> = [
    ...snapshot.text_attempts.map((attempt) => attempt.llm_usage),
    ...snapshot.voice_attempts.flatMap((attempt) => [attempt.stt_usage, attempt.llm_usage]),
  ];
  const estimated = sumKnownCost(usages, "estimated_cost_microunits");
  const charged = sumKnownCost(usages, "provider_charge_microunits");
  const parts: string[] = [];
  if (estimated !== null) parts.push(`оценка: ${estimated} μunits`);
  if (charged !== null) parts.push(`провайдер: ${charged} μunits`);
  metricCost.textContent = parts.length > 0
    ? parts.join(" · ")
    : "провайдер не сообщил стоимость";
};

const showEvidence = (value: unknown): void => {
  evidenceNode.textContent = JSON.stringify(value, null, 2);
  if (isSessionEvidenceSnapshot(value)) renderTelemetry(value);
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

const waitForVoiceEvents = async (
  requestSequence: number,
  onSegment: (segment: VoiceSegment) => void,
): Promise<VoiceResult> => {
  let finalResult: VoiceResult | null = null;
  while (true) {
    const batch = await api<VoiceEventsResponse>("/api/voice/events", {
      request_sequence: requestSequence,
    });
    for (const event of batch.events) {
      if (event.kind === "segment") {
        onSegment(event.segment);
      } else if (event.kind === "complete") {
        finalResult = event.result;
      } else {
        throw new Error(event.code);
      }
    }
    if (batch.terminal) {
      if (!finalResult) throw new Error("VOICE_STREAM_INCOMPLETE");
      return finalResult;
    }
  }
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

const collectPlayoutTimestamps = (
  stats: RTCStatsReport | undefined,
  expectedKind?: "audio" | "video",
): number[] => {
  const timestamps: number[] = [];
  stats?.forEach((raw) => {
    const stat = raw as unknown as InboundRtpSyncStat;
    if (stat.type !== "inbound-rtp" || !Number.isFinite(stat.estimatedPlayoutTimestamp)) return;
    const packetsReceived = stat.packetsReceived;
    if (packetsReceived === undefined || !Number.isFinite(packetsReceived) || packetsReceived <= 0) return;
    const kind = stat.kind ?? stat.mediaType;
    if (expectedKind && kind !== undefined && kind !== expectedKind) return;
    timestamps.push(stat.estimatedPlayoutTimestamp as number);
  });
  return timestamps;
};

const readAvSyncOffsetMillis = async (): Promise<number | null> => {
  const currentPeer = peer;
  let audio: number[] = [];
  let videoOffsets: number[] = [];
  if (currentPeer) {
    const stats = await currentPeer.getStats();
    audio = collectPlayoutTimestamps(stats, "audio");
    videoOffsets = collectPlayoutTimestamps(stats, "video");
  } else {
    const audioStats = liveKitAudioTrack?.getRTCStatsReport;
    const videoStats = liveKitVideoTrack?.getRTCStatsReport;
    if (!audioStats || !videoStats) return null;
    const [audioReport, videoReport] = await Promise.all([
      audioStats.call(liveKitAudioTrack),
      videoStats.call(liveKitVideoTrack),
    ]);
    audio = collectPlayoutTimestamps(audioReport, "audio");
    videoOffsets = collectPlayoutTimestamps(videoReport, "video");
  }
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
        void postMediaEvidence("audio_started", voice.audioStartedElapsed, voice.requestSequence)
          .catch(() => undefined);
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
  const transportReady = realtimeReadiness.control && backendStatus.session_state === "active";
  const textReady = backendStatus.conversation_readiness !== "none";
  const voiceReady = backendStatus.conversation_readiness === "text_and_voice"
    && realtimeReadiness.audio;
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

const handleProviderClientEvent = (raw: string): void => {
  if (!raw) return;
  void api<ClientEvent | null>("/api/avatar/client-event", { message: raw })
    .then((normalized) => {
      if (normalized?.kind === "playback_started") {
        providerPlaybackId = normalized.playback_id;
      } else if (normalized?.kind === "playback_done") {
        providerPlaybackId = null;
        voiceCommandScheduler.playbackDone();
      }
      updateControls();
    })
    .catch(() => undefined);
};

const dispatchClientCommand = async (command: ClientCommand): Promise<void> => {
  if (command.route.kind === "web_rtc_data_channel") {
    const channel = providerDataChannel;
    if (
      !channel
      || channel.readyState !== "open"
      || channel.label !== command.route.label
    ) {
      throw new Error("CLIENT_TRANSPORT_UNAVAILABLE");
    }
    channel.send(command.payload);
    return;
  }
  const room = liveKitRoom;
  if (!room) throw new Error("CLIENT_TRANSPORT_UNAVAILABLE");
  await room.localParticipant.sendText(command.payload, { topic: command.route.topic });
};

const voiceCommandScheduler = new PlaybackAwareCommandScheduler<ClientCommand>(
  dispatchClientCommand,
);

const attachLiveKitTrack = (track: LiveKitTrack): void => {
  if (track.kind === "video") {
    liveKitVideoTrack = track;
    track.attach(video);
    realtimeReadiness.video = true;
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
    updateControls();
  } else if (track.kind === "audio") {
    liveKitAudioTrack = track;
    realtimeReadiness.audio = true;
    track.attach(avatarAudio);
    if (track.mediaStreamTrack) {
      void attachRemoteAudioEvidence(track.mediaStreamTrack).catch(() => undefined);
    }
    updateControls();
  }
};

const detachLiveKitTrack = (track: LiveKitTrack, element: HTMLMediaElement): void => {
  try {
    track.detach?.(element);
  } catch {
    // The media element is still cleared below; provider detach is best-effort cleanup.
  }
  element.srcObject = null;
};

const handleLiveKitTrackUnsubscribed = (track: LiveKitTrack): void => {
  if (track === liveKitAudioTrack) {
    detachLiveKitTrack(track, avatarAudio);
    liveKitAudioTrack = null;
    realtimeReadiness.audio = false;
    stopMicrophoneCapture();
    setStatus(
      "Аудиопоток аватара потерян. Голос временно недоступен; текст остаётся доступен.",
      "error",
    );
    if (voiceRequestInFlight || voiceCommandScheduler.hasActivePlayback) {
      void interruptAvatar();
    }
  }
  if (track === liveKitVideoTrack) {
    detachLiveKitTrack(track, video);
    liveKitVideoTrack = null;
    realtimeReadiness.video = false;
    stage?.classList.remove("has-video");
    setStatus(
      "Видео-поток аватара потерян. Голос остаётся доступен; ожидаю восстановление LiveKit…",
      "error",
    );
  }
  updateControls();
};

const clearRealtimeMedia = (): void => {
  realtimeReadiness = { control: false, audio: false, video: false };
  liveKitAudioTrack = null;
  liveKitVideoTrack = null;
  video.srcObject = null;
  avatarAudio.srcObject = null;
  stage?.classList.remove("has-video");
  updateControls();
};

const closePeerTransport = (): void => {
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

const handleUnexpectedLiveKitDisconnect = async (room: LiveKitRoom): Promise<void> => {
  if (liveKitRoom !== room) return;
  liveKitRoom = null;
  stopMicrophoneCapture();
  stopRemoteEvidence();
  clearRealtimeMedia();
  setStatus("LiveKit отключен. Завершаю зависшую сессию…", "error");
  if (!backendSessionPresent()) return;
  try {
    await api<{ ok: true }>("/api/session/close", {});
    await syncStatus();
    await refreshSessionEvidence();
    setStatus(
      "LiveKit отключен. Сессия закрыта — сохраните evidence snapshot и подключитесь снова.",
      "error",
    );
  } catch (error) {
    await syncStatus().catch(() => undefined);
    setStatus(
      error instanceof Error
        ? `LiveKit отключен; cleanup: ${error.message}`
        : "LiveKit отключен; cleanup failed",
      "error",
    );
  }
};

const connectWebRtcTransport = async (
  transport: Extract<RealtimeTransport, { kind: "web_rtc" }>,
  clientControl: ClientControl | null,
): Promise<void> => {
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
      realtimeReadiness.audio = true;
      void attachRemoteAudioEvidence(event.track).catch(() => undefined);
    }
    updateControls();
  };
  peer.onconnectionstatechange = () => {
    if (!peer) return;
    const state = peer.connectionState;
    realtimeReadiness.control = state === "connected";
    updateControls();
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

  await peer.setRemoteDescription({ type: transport.offer.kind, sdp: transport.offer.sdp });
  const answer = await peer.createAnswer();
  await peer.setLocalDescription(answer);
  await api<{ ok: true }>("/api/avatar/answer", { kind: answer.type, sdp: answer.sdp ?? "" });
  answerSubmitted = true;
  await flushIce();
};

const connectLiveKitTransport = async (
  transport: Extract<RealtimeTransport, { kind: "live_kit" }>,
): Promise<void> => {
  const sdk = await loadLiveKitSdk();
  const room = new sdk.Room();
  liveKitRoom = room;
  room.on(sdk.RoomEvent.TrackSubscribed, (...args: unknown[]) => {
    const track = args[0] as LiveKitTrack | undefined;
    if (track) attachLiveKitTrack(track);
  });
  room.on(sdk.RoomEvent.TrackUnsubscribed, (...args: unknown[]) => {
    const track = args[0] as LiveKitTrack | undefined;
    if (track) handleLiveKitTrackUnsubscribed(track);
  });
  room.on(sdk.RoomEvent.DataReceived, (...args: unknown[]) => {
    const payload = args[0];
    if (!(payload instanceof Uint8Array)) return;
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
  room.on(sdk.RoomEvent.Disconnected, () => {
    void handleUnexpectedLiveKitDisconnect(room);
  });
  await room.connect(transport.server_url, transport.token);
  realtimeReadiness.control = true;
  updateControls();
};

const connectAvatar = async (): Promise<void> => {
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
    const audience = audienceSelect.value as SessionAudience;
    const start = await api<StartResponse>("/api/avatar/start", { consent: true, audience });
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
    } else {
      await connectLiveKitTransport(start.transport);
    }

    updateControls();
    const transportName = start.transport.kind === "web_rtc" ? "WebRTC" : "LiveKit";
    setStatus(
      selectedAudience() === "visitor"
        ? `Visitor-сессия ${transportName} согласована`
        : `${transportName} согласован`,
      "ready",
    );
    showEvidence({ transport: start.transport.kind, capabilities: [...capabilities] });
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
  setStatus("Распознаю и начинаю ответ…");
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

    const started = await apiBinary<VoiceStartAck>("/api/voice/turn", pcm, requestSequence);
    if (started.request_sequence !== requestSequence) throw new Error("VOICE_STREAM_SEQUENCE_MISMATCH");

    let deliveryFailure: Error | null = null;
    const scheduleSegmentDelivery = (segment: VoiceSegment): void => {
      if (!segment.client_command) return;
      void (async () => {
        const sent = await voiceCommandScheduler.dispatch(segment.client_command);
        if (!sent) return;
        await api<{ ok: true }>("/api/avatar/client-delivery-sent", {
          evidence_turn_sequence: segment.evidence_turn_sequence,
          evidence_output_sequence: segment.evidence_output_sequence,
        });
      })().catch((error: unknown) => {
        deliveryFailure = error instanceof Error ? error : new Error("CLIENT_TRANSPORT_UNAVAILABLE");
        voiceCommandScheduler.interrupt();
        void api<{ ok: true }>("/api/avatar/interrupt", {}).catch(() => undefined);
        if (activeVoiceEvidence?.requestSequence === requestSequence) {
          setStatus(deliveryFailure.message, "error");
        }
      });
    };

    const result = await waitForVoiceEvents(requestSequence, scheduleSegmentDelivery);
    if (deliveryFailure) throw deliveryFailure;

    const voice = activeVoiceEvidence;
    if (voice?.requestSequence === requestSequence) {
      voice.responseComplete = true;
      if (voice.audioStartedElapsed !== null) {
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
    if (recording) {
      await finishMicrophoneTurn();
    } else {
      if (voiceCommandScheduler.hasActivePlayback) {
        await interruptAvatar();
      }
      await startMicrophone();
    }
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
      // Cancel the canonical turn first. This stops the provider stream and releases the runtime
      // engine lock before a provider-specific browser interrupt command is prepared.
      await api<{ ok: true }>("/api/avatar/interrupt", {});
    }
    if (clientReady) {
      const command = await api<ClientCommand>("/api/avatar/client-interrupt", {
        playback_id: playbackId,
      });
      await dispatchClientCommand(command);
      providerPlaybackId = null;
      updateControls();
      await refreshSessionEvidence();
      return;
    }
    if (!voiceRequestInFlight) {
      await api<{ ok: true }>("/api/avatar/interrupt", {});
    }
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
