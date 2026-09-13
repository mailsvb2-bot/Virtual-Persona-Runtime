type Bootstrap = { csrf_token: string; egress_enabled: boolean };
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
const statusNode = byId<HTMLElement>("status");
const evidenceNode = byId<HTMLElement>("evidence");

let csrfToken = "";
let peer: RTCPeerConnection | null = null;
let answerSubmitted = false;
let pendingIce: IceCandidatePayload[] = [];
let capabilities = new Set<string>();

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

const closePeer = (): void => {
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

const connectAvatar = async (): Promise<void> => {
  if (!consent.checked) {
    setStatus("Нужно явное согласие", "error");
    return;
  }
  connectButton.disabled = true;
  setStatus("Создаю защищённую сессию…");
  try {
    const start = await api<StartResponse>("/api/avatar/start", { consent: true });
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

    speakButton.disabled = !capabilities.has("text");
    interruptButton.disabled = !capabilities.has("interrupt");
    revokeButton.disabled = false;
    closeButton.disabled = false;
    setStatus("WebRTC согласован", "ready");
    showEvidence({ connectionState: peer.connectionState, capabilities: [...capabilities] });
  } catch (error) {
    closePeer();
    setStatus(error instanceof Error ? error.message : "Ошибка подключения", "error");
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
  try {
    await api<{ ok: true }>(`/api/session/${kind}`, {});
    setStatus(kind === "revoke" ? "Доступ отозван" : "Сессия закрыта", "idle");
  } catch (error) {
    setStatus(error instanceof Error ? error.message : "Ошибка завершения", "error");
  } finally {
    closePeer();
  }
};

connectButton.addEventListener("click", () => void connectAvatar());
speakButton.addEventListener("click", () => void speak());
interruptButton.addEventListener("click", () => void api("/api/avatar/interrupt", {}).catch((error: unknown) => setStatus(String(error), "error")));
revokeButton.addEventListener("click", () => void endSession("revoke"));
closeButton.addEventListener("click", () => void endSession("close"));
window.addEventListener("beforeunload", () => peer?.close());

void api<Bootstrap>("/api/bootstrap")
  .then((bootstrap) => {
    csrfToken = bootstrap.csrf_token;
    if (!bootstrap.egress_enabled) {
      setStatus("Egress выключен на backend", "error");
      connectButton.disabled = true;
    } else {
      setStatus("Готов к подключению");
    }
    return api("/api/status");
  })
  .then(showEvidence)
  .catch((error: unknown) => setStatus(error instanceof Error ? error.message : "Ошибка bootstrap", "error"));
