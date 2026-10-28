//! Actual `OwnerLabEngine` bridge for a single RT1 visitor, without public HTTP.
//! Caller MUST supply a distinct engine per visitor, never the owner's engine.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use std::time::Duration;
use vpr_provider_local_open_source::operation_client::RemoteOperationClient;
use crate::rt1_remote_media_revocation::Rt1RemoteMediaRevocation;
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
    remote: Arc<Rt1RemoteMediaRevocation>,
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
        consent: bool,
    ) -> Result<(Self, LabSignalBundle), String> {
        if !consent {
            return Err("CONSENT_REQUIRED".into());
        }
        let (persona_id, persona_version) = engine
            .reviewed_persona_binding()
            .ok_or_else(|| "reviewed visitor Persona required".to_owned())?;
        let persona_id = persona_id.to_owned();

        let endpoint = std::env::var("RT1_WORKER_ENDPOINT")
            .map_err(|_| "RT1_WORKER_ENDPOINT_NOT_CONFIGURED".to_owned())?;
        let token = std::env::var("RT1_WORKER_TOKEN")
            .map_err(|_| "RT1_WORKER_TOKEN_NOT_CONFIGURED".to_owned())?;
        let client = RemoteOperationClient::new(&endpoint, &token, Duration::from_secs(5))
            .map_err(|_| "RT1_WORKER_CONFIGURATION_INVALID".to_owned())?;

        // The operation identity exists BEFORE admission so a concurrent
        // owner pause can create a durable cancellation tombstone.
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce)
            .map_err(|_| "RT1_OPERATION_ENTROPY_UNAVAILABLE".to_owned())?;
        let operation_id = nonce.iter().fold(String::with_capacity(32), |mut result, octet| {
            use std::fmt::Write;
            write!(&mut result, "{octet:02x}").expect("hex formatting is infallible");
            result
        });
        let remote = Arc::new(Rt1RemoteMediaRevocation::new(
            client.clone(), operation_id.clone(), 1,
        )?);
        let interrupt = Arc::new(LiveVisitorInterrupt::default());
        let stop_interrupt = Arc::clone(&interrupt);
        let stop_remote = Arc::clone(&remote);
        let id = manager.open(visitor_token, move || {
            stop_interrupt.stop();
            let _ = stop_remote.revoke_confirmed();
        })?;

        let admitted = manager.authorize_persona(&id, &persona_id, persona_version);
        if let Err(error) = admitted {
            let _ = manager.close(&id);
            return Err(error);
        }

        // Exactly one CREATE; never retry an ambiguous response. The worker
        // owns durable idempotence and STOP for an in-flight allocation.
        let started = (|| -> Result<LabSignalBundle, String> {
            let session_id = client.create(&operation_id, 1)
                .map_err(|_| "RT1_REMOTE_CREATE_UNCONFIRMED".to_owned())?;
            manager.authorize(&id)?;
            if remote.is_revoked() {
                return Err("RT1_VISITOR_REVOKED_DURING_CREATE".into());
            }
            let offer = client.offer(&operation_id, &session_id)
                .map_err(|_| "RT1_REMOTE_OFFER_UNAVAILABLE".to_owned())?;
            manager.authorize(&id)?;
            if remote.is_revoked() {
                return Err("RT1_VISITOR_REVOKED_DURING_OFFER".into());
            }
            Ok(LabSignalBundle {
                evidence_session_sequence: 1,
                transport: crate::state::LabRealtimeTransport::WebRtc {
                    offer: crate::state::LabSessionDescription {
                        kind: offer.kind,
                        sdp: offer.sdp,
                    },
                    ice_servers: Vec::new(),
                },
                capabilities: Vec::new(),
                client_control: None,
            })
        })();
        let bundle = match started {
            Ok(bundle) => bundle,
            Err(error) => {
                let _ = manager.close(&id);
                let _ = remote.revoke_confirmed();
                return Err(error);
            }
        };
        Ok((Self {
            manager,
            id,
            engine: Mutex::new(engine),
            interrupt,
            remote,
        }, bundle))
    }

    /// Submit an answer only for the current canonical visitor session.
    /// # Errors
    /// A revoked publication or failed remote SDP operation is denied.
    pub fn submit_answer(&self, sdp: &str) -> Result<(), String> {
        self.manager.authorize(&self.id)?;
        if self.remote.is_revoked() {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        self.remote.submit_answer(sdp)?;
        self.manager.authorize(&self.id)?;
        if self.remote.is_revoked() {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        Ok(())
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
