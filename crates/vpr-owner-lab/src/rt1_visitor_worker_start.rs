//! Verified local-worker voice startup uses the existing canonical engine
//! and owner/visitor authority; no second executor or publication route.
use super::*;

impl Rt1VisitorConversation {
    /// Use the existing owner-side STT/LLM/TTS engine with the authenticated
    /// dual-track worker STOP receipt already bound to this visitor.
    ///
    /// Register the retry handle AFTER visitor admission and BEFORE CREATE.
    /// A rejected visitor cannot exhaust the durable remote cleanup registry.
    ///
    /// # Errors
    /// Denies stale publication, missing worker STOP capacity and uncertain CREATE.
    pub fn start_with_verified_worker(
        manager: Arc<VisitorSessionManager>,
        visitor_token: String,
        engine: OwnerLabEngine,
        consent: bool,
        remote: Arc<Rt1RemoteMediaRevocation>,
        register_cleanup: impl Fn(&Arc<Rt1RemoteMediaRevocation>) -> Result<(), String>,
    ) -> Result<(Self, LabSignalBundle), String> {
        Self::start_inner(
            manager, visitor_token, engine, consent, Some(remote),
            Some(&register_cleanup),
        )
    }
}
