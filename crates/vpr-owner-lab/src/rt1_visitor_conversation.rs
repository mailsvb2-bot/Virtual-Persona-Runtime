//! Actual `OwnerLabEngine` bridge for a single RT1 visitor, without public HTTP.
//! Caller MUST supply a distinct engine per visitor, never the owner's engine.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use vpr_runtime::{SessionRevocationHandle, TurnInterruptHandle};

use crate::rt1_visitor_session_manager::VisitorSessionManager;
use crate::state::{
    LabRealtimeTransport, LabSignalBundle, LabTextResult, LabVoiceResult, LabVoiceSegment,
    OwnerLabEngine, OwnerLabStartRequest,
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
        mut engine: OwnerLabEngine,
        consent: bool,
    ) -> Result<(Self, LabSignalBundle), String> {
        // The reviewed engine must match the durable Persona admitted by the
        // server, not a previously cached owner profile or corrected revision.
        let (persona_id, persona_version) = engine
            .reviewed_persona_binding()
            .ok_or_else(|| "reviewed visitor Persona required".to_owned())?;
        let persona_id = persona_id.to_owned();
        let interrupt = Arc::new(LiveVisitorInterrupt::default());
        let callback = Arc::clone(&interrupt);
        let id = manager.open(visitor_token, move || callback.stop())?;
        if let Err(error) = manager.authorize_persona(&id, &persona_id, persona_version) {
            let _ = manager.close(&id);
            return Err(error);
        }
        // The provider's initial remote session creation is external egress.
        // Serialize its final authorization and dispatch against owner pause.
        // Never call the provider from an unfenced pre/post authorization gap.
        let started = manager.provider_dispatch(&id, || {
            engine
                .start_visitor(OwnerLabStartRequest { consent })
                .map_err(|error| error.code().to_owned())
        });
        let bundle = match started {
            Ok(bundle) => bundle,
            Err(error) => {
                let _ = manager.close(&id);
                return Err(error);
            }
        };
        // WebRTC needs Answer/ICE provider operations. This wrapper does not
        // yet expose an epoch-fenced signaling path, so never return an offer
        // that a caller cannot safely complete.
        if matches!(&bundle.transport, LabRealtimeTransport::WebRtc { .. }) {
            let _ = manager.close(&id);
            let _ = engine.revoke();
            return Err("RT1 WebRTC signaling not yet supported".into());
        }
        // A pause can happen during the remote avatar creation. The callback
        // sets cancelled even before the real runtime handle is available.
        let revoker = engine.session_revocation_handle();
        *interrupt.session.lock() = revoker;
        if interrupt.cancelled.load(Ordering::Acquire)
            || manager
                .authorize_persona(&id, &persona_id, persona_version)
                .is_err()
        {
            interrupt.stop();
            let _ = engine.revoke();
            let _ = manager.close(&id);
            return Err("visitor authorization revoked during startup".into());
        }
        Ok((
            Self {
                manager,
                id,
                engine: Mutex::new(engine),
                interrupt,
            },
            bundle,
        ))
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
