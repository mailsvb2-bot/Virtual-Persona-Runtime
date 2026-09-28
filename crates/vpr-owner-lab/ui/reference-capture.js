const byId = (id) => {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing element ${id}`);
  return element;
};

const rights = byId("reference-rights");
const voiceInput = byId("voice-reference-file");
const appearanceInput = byId("appearance-reference-file");
const voiceUpload = byId("voice-reference-upload");
const appearanceUpload = byId("appearance-reference-upload");
const voiceRecord = byId("voice-reference-record");
const voiceClear = byId("voice-reference-clear");
const appearanceClear = byId("appearance-reference-clear");
const voiceStatus = byId("voice-reference-status");
const appearanceStatus = byId("appearance-reference-status");
const microphoneSelect = document.getElementById("microphone-device");

let csrfToken = "";
let recorder = null;
let recorderStream = null;
let recorderChunks = [];
let recorderTimer = null;

const setReferenceStatus = (kind, text, state = "idle") => {
  const node = kind === "voice" ? voiceStatus : appearanceStatus;
  node.textContent = text;
  node.dataset.state = state;
};

const requireRights = () => {
  if (!rights.checked) throw new Error("Сначала подтвердите права и согласие на использование reference.");
};

const apiJson = async (path, body) => {
  const response = await fetch(path, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-VPR-CSRF": csrfToken,
    },
    body: JSON.stringify(body),
    credentials: "same-origin",
    cache: "no-store",
  });
  const payload = await response.json().catch(() => ({ code: "INVALID_RESPONSE" }));
  if (!response.ok) throw new Error(payload.code ?? `HTTP_${response.status}`);
  return payload;
};

const uploadBlob = async (kind, blob, mediaType) => {
  requireRights();
  if (!csrfToken) throw new Error("REFERENCE_BOOTSTRAP_PENDING");
  if (!blob.size) throw new Error("REFERENCE_EMPTY");
  setReferenceStatus(kind, "Проверяю reference…");
  const response = await fetch("/api/references/intake", {
    method: "POST",
    headers: {
      "Content-Type": "application/octet-stream",
      "X-VPR-CSRF": csrfToken,
      "X-VPR-Reference-Kind": kind,
      "X-VPR-Reference-Media-Type": mediaType,
      "X-VPR-Reference-Consent": "confirmed",
    },
    body: blob,
    credentials: "same-origin",
    cache: "no-store",
  });
  const payload = await response.json().catch(() => ({ code: "INVALID_RESPONSE" }));
  if (!response.ok) throw new Error(payload.code ?? `HTTP_${response.status}`);
  const shortHash = String(payload.sha256 ?? "").slice(0, 12);
  setReferenceStatus(
    kind,
    `Принят локально · ${payload.bytes} байт · SHA-256 ${shortHash}… · raw не хранится`,
    "ready",
  );
};

const refresh = async () => {
  const response = await fetch("/api/references", {
    credentials: "same-origin",
    cache: "no-store",
  });
  if (!response.ok) return;
  const snapshot = await response.json();
  for (const kind of ["voice", "appearance"]) {
    const item = snapshot[kind];
    if (item) {
      setReferenceStatus(
        kind,
        `Принят локально · ${item.bytes} байт · SHA-256 ${String(item.sha256).slice(0, 12)}… · raw не хранится`,
        "ready",
      );
    } else {
      setReferenceStatus(kind, "Reference ещё не выбран.");
    }
  }
};

const uploadFile = async (kind, input) => {
  const file = input.files?.[0];
  if (!file) throw new Error("REFERENCE_FILE_REQUIRED");
  const mediaType = file.type.trim().toLowerCase();
  if (!mediaType) throw new Error("REFERENCE_MEDIA_TYPE_UNKNOWN");
  await uploadBlob(kind, file, mediaType);
};

const clearReference = async (kind) => {
  if (!csrfToken) throw new Error("REFERENCE_BOOTSTRAP_PENDING");
  await apiJson("/api/references/clear", { kind });
  setReferenceStatus(kind, "Reference очищен.");
};

const stopRecorderTracks = () => {
  recorderStream?.getTracks().forEach((track) => track.stop());
  recorderStream = null;
  if (recorderTimer !== null) window.clearTimeout(recorderTimer);
  recorderTimer = null;
};

const finishRecording = () => {
  if (recorder?.state === "recording") recorder.stop();
};

const startRecording = async () => {
  requireRights();
  if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === "undefined") {
    throw new Error("REFERENCE_RECORDING_UNAVAILABLE");
  }
  const selectedDeviceId = microphoneSelect instanceof HTMLSelectElement
    ? microphoneSelect.value.trim()
    : "";
  recorderStream = await navigator.mediaDevices.getUserMedia({
    audio: selectedDeviceId ? { deviceId: { exact: selectedDeviceId } } : true,
  });
  recorderChunks = [];
  const preferred = ["audio/webm;codecs=opus", "audio/webm", "audio/ogg"]
    .find((type) => MediaRecorder.isTypeSupported?.(type));
  recorder = preferred
    ? new MediaRecorder(recorderStream, { mimeType: preferred })
    : new MediaRecorder(recorderStream);
  recorder.addEventListener("dataavailable", (event) => {
    if (event.data.size) recorderChunks.push(event.data);
  });
  recorder.addEventListener("stop", () => {
    const mediaType = recorder?.mimeType || recorderChunks[0]?.type || "audio/webm";
    const blob = new Blob(recorderChunks, { type: mediaType });
    stopRecorderTracks();
    recorder = null;
    voiceRecord.textContent = "Записать голос (до 15 с)";
    void uploadBlob("voice", blob, mediaType).catch((error) => {
      setReferenceStatus("voice", error instanceof Error ? error.message : "REFERENCE_UPLOAD_FAILED", "error");
    });
  }, { once: true });
  recorder.start();
  voiceRecord.textContent = "Остановить запись";
  setReferenceStatus("voice", "Идёт локальная запись… raw будет отброшен после intake.", "ready");
  recorderTimer = window.setTimeout(finishRecording, 15_000);
};

voiceUpload.addEventListener("click", () => {
  void uploadFile("voice", voiceInput).catch((error) => {
    setReferenceStatus("voice", error instanceof Error ? error.message : "REFERENCE_UPLOAD_FAILED", "error");
  });
});
appearanceUpload.addEventListener("click", () => {
  void uploadFile("appearance", appearanceInput).catch((error) => {
    setReferenceStatus("appearance", error instanceof Error ? error.message : "REFERENCE_UPLOAD_FAILED", "error");
  });
});
voiceClear.addEventListener("click", () => {
  void clearReference("voice").catch((error) => {
    setReferenceStatus("voice", error instanceof Error ? error.message : "REFERENCE_CLEAR_FAILED", "error");
  });
});
appearanceClear.addEventListener("click", () => {
  void clearReference("appearance").catch((error) => {
    setReferenceStatus("appearance", error instanceof Error ? error.message : "REFERENCE_CLEAR_FAILED", "error");
  });
});
voiceRecord.addEventListener("click", () => {
  if (recorder?.state === "recording") {
    finishRecording();
    return;
  }
  void startRecording().catch((error) => {
    stopRecorderTracks();
    recorder = null;
    voiceRecord.textContent = "Записать голос (до 15 с)";
    setReferenceStatus("voice", error instanceof Error ? error.message : "REFERENCE_RECORDING_FAILED", "error");
  });
});

window.addEventListener("vpr:bootstrap", (event) => {
  const detail = event instanceof CustomEvent ? event.detail : null;
  const token = detail && typeof detail.csrfToken === "string" ? detail.csrfToken : "";
  if (!token) {
    setReferenceStatus("voice", "REFERENCE_BOOTSTRAP_FAILED", "error");
    setReferenceStatus("appearance", "REFERENCE_BOOTSTRAP_FAILED", "error");
    return;
  }
  csrfToken = token;
  void refresh().catch((error) => {
    const message = error instanceof Error ? error.message : "REFERENCE_REFRESH_FAILED";
    setReferenceStatus("voice", message, "error");
    setReferenceStatus("appearance", message, "error");
  });
}, { once: true });
