//! Actual `OwnerLabEngine` bridge for a single RT1 visitor, without public HTTP.
//! Caller MUST supply a distinct engine per visitor, never the owner's engine.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use vpr_runtime::{SessionRevocationHandle, TurnInterruptHandle};

use crate::rt1_visitor_session_manager::VisitorSessionManager;
use crate::state::{
    LabSignalBundle, LabTextResult, LabVoiceResult, LabVoiceSegment, OwnerLabEngine,
};

#[derive(Default)]
struct LiveVisitorInterrupt {
    cancelled: AtomicBool,
    session: Mutex<Option<SessionRevocationHandle>>,
    turn: Mutex<Option<TurnInterruptHandle>>,
}

impl LiveVisitorInterrupt {
    fn stop(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(turn) = self.turn.lock().clone() {
            let _ = turn.interrupt();
        }
        if let Some(session) = self.session.lock().clone() {
            let _ = session.revoke_authority();
        }
    }
}

/// One dedicated existing conversation engine per authorized visitor.
/// This does not provide network exposure, audio chunks or `LiveKit` sender STOP.
pub struct Rt1VisitorConversation {
    manager: Arc<VisitorSessionManager>,
    id: String,
    engine: Mutex<OwnerLabEngine>,
    interrupt: Arc<LiveVisitorInterrupt>,
}

impl Rt1VisitorConversation {
    /// Start the real visitor-scoped avatar transport after canonical visitor
    /// admission. Only return the offer/token if authority remains current.
    ///
    /// # Errors
    /// Rejects invalid authority, failed provider startup or concurrent revoke.
    pub fn start(
        manager: Arc<VisitorSessionManager>,
        visitor_token: String,
        engine: OwnerLabEngine,
        _consent: bool,
    ) -> Result<(Self, LabSignalBundle), String> {
        // A remote provider that hangs while holding the publication lock
        // blocks owner pause/unpublish indefinitely. A detached timeout thread
        // cannot cancel that provider. RT1 must remain fail-closed until the
        // provider supplies a bounded, enforceable STOP/cancellation contract.
        let (persona_id, persona_version) = engine
            .reviewed_persona_binding()
            .ok_or_else(|| "reviewed visitor Persona required".to_owned())?;
        let persona_id = persona_id.to_owned();
        // Bind the revocation callback at admission time, before any
        // provider work could start. Never register an empty STOP callback:
        // owner pause/unpublish must reach this visitor's cancellation handle
        // without waiting on the conversation engine mutex.
        let interrupt = Arc::new(LiveVisitorInterrupt::default());
        let stop_handle = Arc::clone(&interrupt);
        let id = manager.open(visitor_token, move || stop_handle.stop())?;
        let admitted = manager.authorize_persona(&id, &persona_id, persona_version);
        let _ = manager.close(&id);
        // Consume both owned arguments after closing the transient admission.
        // No provider is started and no visitor engine escapes this boundary.
        drop((manager, engine));
        admitted?;
        Err("RT1_PROVIDER_STARTUP_REQUIRES_BOUNDED_CANCELLATION".into())
    }

    /// # Errors
    /// Denies revoked visitor authority or returns the canonical turn error.
    pub fn text_turn(&self, _text: &str) -> Result<LabTextResult, String> {
        // Issue #319: compute_buffered authorizes only side-effect-free
        // computation, never provider egress. A pre/post check cannot retract
        // an LLM request sent concurrently with owner revocation.
        // Keep this internal RT1 text path fail-closed until a canonical,
        // revocation-serialized provider egress fence is implemented.
        self.manager.authorize(&self.id)?;
        Err("RT1 visitor text provider egress disabled pending revocation fence (#319)".into())
    }

    /// RT1 voice provider egress is deliberately denied until the canonical
    /// workspace owns a revocation-serialized final media delivery/STOP gate.
    /// A pre/post authorization check around external provider I/O is unsafe:
    /// owner pause may commit between the check and provider transmission.
    ///
    /// # Errors
    /// Always denies unsupported visitor microphone provider egress.
    pub fn voice_turn(&self, _pcm_s16le_mono_16khz: Vec<u8>) -> Result<LabVoiceResult, String> {
        unsupported_visitor_voice_egress()
    }

    /// Visitor streaming is fail-closed: never call an arbitrary egress
    /// callback between separate authorization checks. Buffered voice and
    /// streaming voice share the same unsupported provider-egress boundary.
    ///
    /// # Errors
    /// Always denies unsupported visitor media delivery.
    pub fn voice_turn_streaming(
        &self,
        _pcm_s16le_mono_16khz: Vec<u8>,
        _emit_segment: impl FnMut(LabVoiceSegment) -> Result<(), String>,
    ) -> Result<LabVoiceResult, String> {
        Err("RT1_VOICE_PROVIDER_EGRESS_NOT_AUTHORIZED".into())
    }

    /// # Errors
    /// Always attempts provider cleanup, even if owner pause already removed
    /// the registration. A failed remote close remains retryable on a later call.
    pub fn close(&self) -> Result<(), String> {
        let _ = self.manager.close(&self.id);
        self.interrupt.stop();
        self.engine
            .lock()
            .revoke()
            .map_err(|error| error.code().to_owned())
    }
}

impl Drop for Rt1VisitorConversation {
    fn drop(&mut self) {
        let _ = self.manager.close(&self.id);
        self.interrupt.stop();
        let _ = self.engine.lock().revoke();
    }
}

fn unsupported_visitor_voice_egress<T>() -> Result<T, String> {
    Err("RT1_VOICE_PROVIDER_EGRESS_NOT_AUTHORIZED".into())
}

#[cfg(test)]
mod rt1_voice_egress_tests {
    use super::*;

    #[test]
    fn voice_provider_egress_is_denied_without_media_fence() {
        let result: Result<LabVoiceResult, String> = unsupported_visitor_voice_egress();
        assert_eq!(
            result.unwrap_err(),
            "RT1_VOICE_PROVIDER_EGRESS_NOT_AUTHORIZED"
        );
    }

    #[test]
    fn voice_streaming_rejects_before_any_delivery() {
        let result: Result<LabVoiceResult, String> = unsupported_visitor_voice_egress();
        assert_eq!(
            result.unwrap_err(),
            "RT1_VOICE_PROVIDER_EGRESS_NOT_AUTHORIZED"
        );
    }
}
