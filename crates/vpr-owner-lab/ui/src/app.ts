type Bootstrap = { csrf_token: string; egress_enabled: boolean };
type LabStatus = { session_state: string; avatar_open: boolean; egress_enabled: boolean; voice_ready: boolean };
type VoiceResult = { transcript: string; reply: string; locale: string; stt_millis: number; llm_millis: number; avatar_millis: number; total_millis: number };
type SessionDescription = { kind: RTCSdpType; sdp: string };
type IceServer = { urls: string[]; username: string | null; credential: string | null };
type StartResponse = { offer: SessionDescription; ice_servers: IceServer[]; capabilities: string[] };
type ErrorPayload = { ok: false; code: string };
type IceCandidatePayload = { candidate: string | null; sdpMid: string | null; sdpMLineIndex: number | null };

const byId = <T extends HTMLElement>(id: string): T => {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing element ${id}`);
  return element as T;
};

const video = byId<HTMLVideoElement>("avatar");
const stage = document.querySelector<HTMLElement>(".stage");
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
let backendStatus: LabStatus = { session_state: "none", avatar_open: false, egress_enabled: false, voice_ready: false };
let peer: RTCPeerConnection | null = null;
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
let voiceRequestInFlight = false;

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

const apiBinary = async <T>(path: string, body: ArrayBuffer): Promise<T> => {
  const response = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/octet-stream", "X-VPR-CSRF": csrfToken },
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

const updateControls = (): void => {
  const transportReady = peer !== null && answerSubmitted && backendStatus.session_state === "active";
  speakButton.disabled = !transportReady || !capabilities.has("text");
  interruptButton.disabled = !voiceRequestInFlight && (!transportReady || !capabilities.has("interrupt"));
  voiceButton.disabled = recording ? false : !transportReady || !backendStatus.voice_ready || voiceRequestInFlight;
  voiceButton.textContent = recording ? "Остановить и отправить" : "Начать говорить";
  revokeButton.disabled = !backendSessionPresent()
    || (backendStatus.session_state === "revoked" && !backendStatus.avatar_open);
  closeButton.disabled = !backendSessionPresent();
  connectButton.disabled = !egressEnabled || backendSessionPresent();
};

const closePeerTransport = (): void => {
  stopMicrophoneCapture();
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
  setStatus("Создаю защищённую сессию…");
  try {
    const start = await api<StartResponse>("/api/avatar/start", { consent: true });
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
      video.srcObject = event.streams[0] ?? new MediaStream([event.track]);
      stage?.classList.add("has-video");
      setStatus("Видео подключено", "ready");
    };
    peer.onconnectionstatechange = () => {
      if (!peer) return;
      showEvidence({ connectionState: peer.connectionState, capabilities: [...capabilities] });
      if (peer.connectionState === "failed") setStatus("WebRTC connection failed", "error");
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
    setStatus("WebRTC согласован", "ready");
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
  recordingTimer = window.setTimeout(() => void finishMicrophoneTurn(), 30_000);
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
  updateControls();
  setStatus("Распознаю и формирую ответ…");
  try {
    const pcm = encodeS16Le(resampleMono(samples, inputRate));
    const result = await apiBinary<VoiceResult>("/api/voice/turn", pcm);
    showEvidence(result);
    setStatus(`Вы: ${result.transcript} · Ответ: ${result.reply}`, "ready");
  } catch (error) {
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
  speakButton.disabled = true;
  try {
    await api<{ ok: true }>("/api/avatar/speak", { text });
    setStatus("Фраза отправлена", "ready");
  } catch (error) {
    setStatus(error instanceof Error ? error.message : "Ошибка отправки", "error");
  } finally {
    speakButton.disabled = !capabilities.has("text") || peer === null;
  }
};

const endSession = async (kind: "revoke" | "close"): Promise<void> => {
  closePeerTransport();
  try {
    await api<{ ok: true }>(`/api/session/${kind}`, {});
    await syncStatus();
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

connectButton.addEventListener("click", () => void connectAvatar());
speakButton.addEventListener("click", () => void speak());
interruptButton.addEventListener("click", () => void api("/api/avatar/interrupt", {}).catch((error: unknown) => setStatus(String(error), "error")));
revokeButton.addEventListener("click", () => void endSession("revoke"));
closeButton.addEventListener("click", () => void endSession("close"));
voiceButton.addEventListener("click", () => void toggleVoice());
window.addEventListener("pagehide", closeBackendOnUnload);

void api<Bootstrap>("/api/bootstrap")
  .then(async (bootstrap) => {
    csrfToken = bootstrap.csrf_token;
    egressEnabled = bootstrap.egress_enabled;
    await syncStatus();
    if (!egressEnabled) {
      setStatus("Egress выключен на backend", "error");
    } else if (backendSessionPresent()) {
      setStatus("Найдена незакрытая сессия — доступно безопасное завершение", "error");
    } else {
      setStatus("Готов к подключению");
    }
    updateControls();
  })
  .catch((error: unknown) => setStatus(error instanceof Error ? error.message : "Ошибка bootstrap", "error"));
