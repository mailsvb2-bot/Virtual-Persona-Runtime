use vpr_domain::RealtimeSessionState;

impl super::OwnerLabEngine {
    /// The reviewed identity used for all subsequent runtime turn snapshots.
    pub(crate) fn reviewed_persona_binding(&self) -> Option<(&str, u64)> {
        self.reviewed_owner_context.as_ref().map(|context| {
            (
                context.identity().id().as_str(),
                context.identity().version().get(),
            )
        })
    }
}

pub(super) const fn state_name(state: RealtimeSessionState) -> &'static str {
    match state {
        RealtimeSessionState::Created => "created",
        RealtimeSessionState::Active => "active",
        RealtimeSessionState::Draining => "draining",
        RealtimeSessionState::Revoked => "revoked",
        RealtimeSessionState::Closed => "closed",
    }
}
