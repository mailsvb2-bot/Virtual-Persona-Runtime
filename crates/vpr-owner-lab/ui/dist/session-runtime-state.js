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
const INITIAL_REALTIME = {
    control: false,
    audio: false,
    video: false,
};
export class SessionRuntimeState {
    backendValue = INITIAL_LAB_STATUS;
    realtimeValue = INITIAL_REALTIME;
    playbackIdValue = null;
    backendRevisionValue = 0;
    get backend() {
        return this.backendValue;
    }
    get realtime() {
        return this.realtimeValue;
    }
    get playbackId() {
        return this.playbackIdValue;
    }
    get backendRevision() {
        return this.backendRevisionValue;
    }
    applyBackend(status) {
        this.backendValue = status;
        this.backendRevisionValue += 1;
        return status;
    }
    setRealtimeReadiness(patch) {
        this.realtimeValue = { ...this.realtimeValue, ...patch };
        return this.realtimeValue;
    }
    setPlaybackId(playbackId) {
        this.playbackIdValue = playbackId;
    }
    resetRealtime() {
        this.realtimeValue = INITIAL_REALTIME;
        this.playbackIdValue = null;
    }
}
