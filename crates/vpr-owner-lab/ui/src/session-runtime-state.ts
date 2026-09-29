export type SessionAudience = "owner" | "visitor";
export type ConversationReadiness = "none" | "text" | "text_and_voice";
export type ModalityReadiness = "not_ready" | "preparing" | "ready" | "failed";
export type ModalityReadinessStatus = {
  text: ModalityReadiness;
  voice: ModalityReadiness;
  video: ModalityReadiness;
};
export type LabStatus = {
  session_state: string;
  avatar_open: boolean;
  egress_enabled: boolean;
  conversation_readiness: ConversationReadiness;
  modality_readiness: ModalityReadinessStatus;
  session_audience: SessionAudience | null;
  owner_context_state: "missing" | "reviewed";
  persona_version: number;
  reviewed_owner_claims: number;
};

const INITIAL_LAB_STATUS: LabStatus = {
  session_state: "none",
  avatar_open: false,
  egress_enabled: false,
  conversation_readiness: "none",
  modality_readiness: { text: "not_ready", voice: "not_ready", video: "not_ready" },
  session_audience: null,
  owner_context_state: "missing",
  persona_version: 1,
  reviewed_owner_claims: 0,
};

export class SessionRuntimeState {
  backend: LabStatus = INITIAL_LAB_STATUS;
  realtime = { control: false, audio: false, video: false };
  playbackId: string | null = null;
  backendRevision = 0;

  applyBackend(status: LabStatus): LabStatus {
    this.backend = status;
    this.backendRevision += 1;
    return status;
  }

  patchBackend(patch: Partial<LabStatus>): LabStatus {
    return this.applyBackend({ ...this.backend, ...patch });
  }

  resetRealtime(): void {
    this.realtime = { control: false, audio: false, video: false };
    this.playbackId = null;
  }
}
