const INITIAL_LAB_STATUS = {
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
    backend = INITIAL_LAB_STATUS;
    realtime = { control: false, audio: false, video: false };
    playbackId = null;
    backendRevision = 0;
    applyBackend(status) {
        this.backend = status;
        this.backendRevision += 1;
        return status;
    }
    patchBackend(patch) {
        return this.applyBackend({ ...this.backend, ...patch });
    }
    resetRealtime() {
        this.realtime = { control: false, audio: false, video: false };
        this.playbackId = null;
    }
}
