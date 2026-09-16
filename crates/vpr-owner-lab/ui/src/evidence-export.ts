type ErrorPayload = { code?: string };

export type ExportedSessionEvidence = {
  session_sequence: number;
  participant_role: "owner" | "visitor";
};

const parseIdentity = (bytes: ArrayBuffer): ExportedSessionEvidence => {
  const text = new TextDecoder().decode(bytes);
  const value = JSON.parse(text) as Partial<ExportedSessionEvidence>;
  if (
    !Number.isSafeInteger(value.session_sequence)
    || (value.session_sequence ?? 0) <= 0
    || !["owner", "visitor"].includes(value.participant_role ?? "")
  ) {
    throw new Error("INVALID_EVIDENCE_EXPORT");
  }
  return value as ExportedSessionEvidence;
};

const errorCode = (bytes: ArrayBuffer, status: number): string => {
  try {
    const value = JSON.parse(new TextDecoder().decode(bytes)) as ErrorPayload;
    return value.code ?? `HTTP_${status}`;
  } catch {
    return `HTTP_${status}`;
  }
};

const bootstrapCsrf = async (): Promise<string> => {
  const response = await fetch("/api/bootstrap", {
    method: "GET",
    credentials: "same-origin",
    cache: "no-store",
  });
  const payload = await response.json() as { csrf_token?: string };
  if (!response.ok || typeof payload.csrf_token !== "string" || payload.csrf_token.length === 0) {
    throw new Error(`HTTP_${response.status}`);
  }
  return payload.csrf_token;
};

export const downloadSessionEvidence = async (): Promise<ExportedSessionEvidence> => {
  const csrfToken = await bootstrapCsrf();
  const response = await fetch("/api/evidence/session/export", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-VPR-CSRF": csrfToken,
    },
    body: "{}",
    credentials: "same-origin",
    cache: "no-store",
  });
  const bytes = await response.arrayBuffer();
  if (!response.ok) throw new Error(errorCode(bytes, response.status));

  const identity = parseIdentity(bytes);
  const url = URL.createObjectURL(new Blob([bytes], { type: "application/json" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = `session-${identity.session_sequence}-${identity.participant_role}.json`;
  link.hidden = true;
  document.body.append(link);
  link.click();
  link.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
  return identity;
};

const exportButton = document.getElementById("export-evidence") as HTMLButtonElement | null;
exportButton?.addEventListener("click", () => {
  exportButton.disabled = true;
  void downloadSessionEvidence()
    .catch(() => undefined)
    .finally(() => { exportButton.disabled = false; });
});
