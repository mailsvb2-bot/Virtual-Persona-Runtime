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

    /// Interrupting a verified worker terminates the whole remote session.
    /// It cannot authorize another voice turn after both RTP senders stop.
    pub(super) fn interrupt_verified_worker(&self) -> Result<(), String> {
        self.manager.authorize(&self.id)?;
        self.interrupt.stop();
        let _ = self.manager.close(&self.id);
        self.remote
            .as_ref()
            .ok_or_else(|| "RT1_VISITOR_INTERRUPT_DENIED".to_owned())?
            .revoke_confirmed()
            .map_err(|_| "RT1_VISITOR_INTERRUPT_STOP_UNCONFIRMED".to_owned())
    }
}
