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
export type RealtimeReadiness = {
  control: boolean;
  audio: boolean;
  video: boolean;
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

const INITIAL_REALTIME: RealtimeReadiness = {
  control: false,
  audio: false,
  video: false,
};

export class SessionRuntimeState {
  private backendValue: LabStatus = INITIAL_LAB_STATUS;
  private realtimeValue: RealtimeReadiness = INITIAL_REALTIME;
  private playbackIdValue: string | null = null;
  private backendRevisionValue = 0;

  get backend(): Readonly<LabStatus> {
    return this.backendValue;
  }

  get realtime(): Readonly<RealtimeReadiness> {
    return this.realtimeValue;
  }

  get playbackId(): string | null {
    return this.playbackIdValue;
  }

  get backendRevision(): number {
    return this.backendRevisionValue;
  }

  applyBackend(status: LabStatus): LabStatus {
    this.backendValue = status;
    this.backendRevisionValue += 1;
    return status;
  }

  setRealtimeReadiness(patch: Partial<RealtimeReadiness>): RealtimeReadiness {
    this.realtimeValue = { ...this.realtimeValue, ...patch };
    return this.realtimeValue;
  }

  setPlaybackId(playbackId: string | null): void {
    this.playbackIdValue = playbackId;
  }

  resetRealtime(): void {
    this.realtimeValue = INITIAL_REALTIME;
    this.playbackIdValue = null;
  }
}
