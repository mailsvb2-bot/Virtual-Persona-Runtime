use vpr_domain::RealtimeSessionState;

pub(super) const fn state_name(state: RealtimeSessionState) -> &'static str {
    match state {
        RealtimeSessionState::Created => "created",
        RealtimeSessionState::Active => "active",
        RealtimeSessionState::Draining => "draining",
        RealtimeSessionState::Revoked => "revoked",
        RealtimeSessionState::Closed => "closed",
    }
}
