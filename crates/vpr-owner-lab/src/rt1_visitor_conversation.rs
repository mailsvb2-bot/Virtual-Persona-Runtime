//! Actual `OwnerLabEngine` bridge for a single RT1 visitor, without public HTTP.
//! Caller MUST supply a distinct engine per visitor, never the owner's engine.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::rt1_remote_media_revocation::Rt1RemoteMediaRevocation;
use parking_lot::Mutex;
use std::time::Duration;
use vpr_provider_local_open_source::operation_client::RemoteOperationClient;
use vpr_runtime::{RealtimeAvatarStopHandle, SessionRevocationHandle, TurnInterruptHandle};

use crate::rt1_visitor_session_manager::VisitorSessionManager;
use crate::state::{
    LabSignalBundle, LabTextResult, LabVoiceResult, LabVoiceSegment, OwnerLabEngine,
    OwnerLabStartRequest, OwnerLabTurnInput,
};

#[derive(Default)]
struct LiveVisitorInterrupt {
    cancelled: AtomicBool,
    session: Mutex<Option<SessionRevocationHandle>>,
    turn: Mutex<Option<TurnInterruptHandle>>,
    backend_stop: Mutex<Option<RealtimeAvatarStopHandle>>,
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
        // Provider STOP can take network time: never block owner pause on it
        // or on the conversation engine mutex. The explicit visitor close
        // still requires a separate confirmed close_session receipt.
        if let Some(stop) = self.backend_stop.lock().take() {
            std::thread::spawn(move || {
                if let Err(error) = stop.stop() {
                    eprintln!("RT1 best-effort backend STOP failed: {error:?}");
                }
            });
        }
    }
}

/// One dedicated canonical conversation engine per authorized visitor.
/// Private Echo may transmit microphone speech only with cancellable runtime
/// turns and an independent stop-only sender capability.
pub struct Rt1VisitorConversation {
    manager: Arc<VisitorSessionManager>,
    id: String,
    engine: Mutex<OwnerLabEngine>,
    interrupt: Arc<LiveVisitorInterrupt>,
    remote: Option<Arc<Rt1RemoteMediaRevocation>>,
}

impl Rt1VisitorConversation {
    /// Start with the configured canonical avatar provider and its existing
    /// WebRTC/LiveKit transport. No synthetic sender is created here.
    ///
    /// # Errors
    /// Rejects stale publication, failed provider allocation or raced revoke.
    pub fn start(
        manager: Arc<VisitorSessionManager>,
        visitor_token: String,
        mut engine: OwnerLabEngine,
        consent: bool,
    ) -> Result<(Self, LabSignalBundle), String> {
        if !consent {
            return Err("CONSENT_REQUIRED".into());
        }
        let (persona_id, persona_version) = engine
            .reviewed_persona_binding()
            .ok_or_else(|| "reviewed visitor Persona required".to_owned())?;
        let persona_id = persona_id.to_owned();
        let interrupt = Arc::new(LiveVisitorInterrupt::default());
        let on_revoke = Arc::clone(&interrupt);
        let id = manager.open(visitor_token, move || on_revoke.stop())?;
        if let Err(error) = manager.authorize_persona(&id, &persona_id, persona_version) {
            let _ = manager.close(&id);
            return Err(error);
        }

        let started = engine.start_visitor(OwnerLabStartRequest { consent });
        let bundle = match started {
            Ok(bundle) => bundle,
            Err(error) => {
                // A failed remote CREATE can still have allocated an avatar.
                // Never treat manager.close() as proof that the remote sender
                // stopped; explicitly require the canonical provider receipt.
                // A provider error can precede establishment of the visitor
                // session, so STOP may not yet have an addressable target.
                // Report the cleanup uncertainty rather than claim a receipt.
                let _ = engine.stop_visitor_provider_confirmed();
                let _ = manager.close(&id);
                return Err(error.code().to_owned());
            }
        };
        *interrupt.session.lock() = engine.session_revocation_handle();
        *interrupt.backend_stop.lock() = engine.visitor_avatar_stop_handle();
        if interrupt.cancelled.load(Ordering::Acquire)
            || manager
                .authorize_persona(&id, &persona_id, persona_version)
                .is_err()
        {
            interrupt.stop();
            let stop_result = engine.stop_visitor_provider_confirmed();
            let _ = manager.close(&id);
            if stop_result.is_err() {
                return Err("RT1_PROVIDER_START_REVOKED_STOP_UNCONFIRMED".into());
            }
            return Err("RT1_VISITOR_REVOKED_DURING_PROVIDER_START".into());
        }
        Ok((
            Self {
                manager,
                id,
                engine: Mutex::new(engine),
                interrupt,
                remote: None,
            },
            bundle,
        ))
    }

    /// Start the real visitor-scoped avatar transport after canonical visitor
    /// admission. Only return the offer/token if authority remains current.
    ///
    /// # Errors
    /// Rejects invalid authority, failed provider startup or concurrent revoke.
    pub fn start_diagnostic(
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
        getrandom::fill(&mut nonce).map_err(|_| "RT1_OPERATION_ENTROPY_UNAVAILABLE".to_owned())?;
        let operation_id = nonce
            .iter()
            .fold(String::with_capacity(32), |mut result, octet| {
                use std::fmt::Write;
                write!(&mut result, "{octet:02x}").expect("hex formatting is infallible");
                result
            });
        let remote = Arc::new(Rt1RemoteMediaRevocation::new(
            client.clone(),
            operation_id.clone(),
            1,
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
            let session_id = client
                .create(&operation_id, 1)
                .map_err(|_| "RT1_REMOTE_CREATE_UNCONFIRMED".to_owned())?;
            manager.authorize(&id)?;
            if remote.is_revoked() {
                return Err("RT1_VISITOR_REVOKED_DURING_CREATE".into());
            }
            let offer = client
                .offer(&operation_id, &session_id)
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
                    ice_servers: offer
                        .ice_servers
                        .into_iter()
                        .map(|server| crate::state::LabIceServer {
                            urls: server.urls,
                            username: server.username,
                            credential: server.credential,
                        })
                        .collect(),
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
        Ok((
            Self {
                manager,
                id,
                engine: Mutex::new(engine),
                interrupt,
                remote: Some(remote),
            },
            bundle,
        ))
    }

    /// Whether the selected canonical avatar exposes an independent
    /// server-owned STOP capability for private microphone speech.
    /// Generic WebRTC/LiveKit text-control avatars remain voice-denied.
    #[must_use]
    pub fn supports_private_voice(&self) -> bool {
        self.remote.is_none()
            && !self.interrupt.cancelled.load(Ordering::Acquire)
            && self.interrupt.backend_stop.lock().is_some()
            && self.manager.authorize(&self.id).is_ok()
    }

    /// Whether this visitor retains the current canonical publication permit.
    /// The caller must drop the browser transport on false, not retry CREATE.
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.interrupt.cancelled.load(Ordering::Acquire)
            && self
                .remote
                .as_ref()
                .is_none_or(|remote| !remote.is_revoked())
            && self.manager.authorize(&self.id).is_ok()
    }

    /// Submit an answer only for the current canonical visitor session.
    /// # Errors
    /// A revoked publication or failed remote SDP operation is denied.
    pub fn submit_answer(&self, sdp: &str) -> Result<(), String> {
        self.manager.authorize(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        if let Some(remote) = &self.remote {
            remote.submit_answer(sdp)?;
        } else {
            self.engine
                .lock()
                .apply(OwnerLabTurnInput::Answer(
                    vpr_integration::WebRtcSessionDescription {
                        kind: "answer".into(),
                        sdp: sdp.to_owned(),
                    },
                ))
                .map_err(|error| error.code().to_owned())?;
        }
        self.manager.authorize(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        Ok(())
    }

    /// Forward ICE exclusively through the existing avatar provider and
    /// the admitted visitor's canonical runtime turn. Diagnostic worker
    /// sessions do not expose this separate signaling channel.
    ///
    /// # Errors
    /// Rejects revoked authority, diagnostic sessions and provider failure.
    pub fn submit_ice(&self, candidate: vpr_integration::WebRtcIceCandidate) -> Result<(), String> {
        self.manager.authorize(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) || self.remote.is_some() {
            return Err("RT1_VISITOR_ICE_DENIED".into());
        }
        self.engine
            .lock()
            .apply(OwnerLabTurnInput::Ice(candidate))
            .map_err(|error| error.code().to_owned())?;
        self.manager.authorize(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
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

    /// Stage a bounded local fragment under canonical publication authority.
    /// This is NOT permission for provider/browser egress. The consumer must
    /// enforce a final revocation-serialized sender/STOP fence.
    ///
    /// # Errors
    /// Denies cancelled visitors, stale authority and full queues.
    pub fn queue_media_fragment<T>(
        &self,
        sender: &std::sync::mpsc::SyncSender<T>,
        fragment: T,
    ) -> Result<(), String> {
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        self.manager
            .enqueue_media_fragment(&self.id, sender, fragment)
    }

    /// Admit one complete browser microphone utterance into the canonical
    /// bounded pre-egress queue, without invoking STT/LLM/TTS or a sender.
    /// The existing RT0 voice contract is S16LE, mono, 16 kHz, max 30s.
    ///
    /// # Errors
    /// Rejects malformed/empty/unbounded audio, stale publication and a full
    /// queue. This operation does NOT authorize external media delivery.
    pub fn stage_microphone_audio(
        &self,
        sender: &std::sync::mpsc::SyncSender<Vec<u8>>,
        pcm_s16le_mono_16khz: Vec<u8>,
    ) -> Result<(), String> {
        validate_microphone_audio(&pcm_s16le_mono_16khz)?;
        if self.remote.is_some() {
            return Err("RT1_DIAGNOSTIC_VOICE_NOT_AUTHORIZED".into());
        }
        self.queue_media_fragment(sender, pcm_s16le_mono_16khz)
    }

    /// RT1 voice provider egress is deliberately denied until the canonical
    /// workspace owns a revocation-serialized final media delivery/STOP gate.
    /// A pre/post authorization check around external provider I/O is unsafe:
    /// owner pause may commit between the check and provider transmission.
    ///
    /// # Errors
    /// Always denies unsupported visitor microphone provider egress.
    pub fn voice_turn(&self, pcm_s16le_mono_16khz: Vec<u8>) -> Result<LabVoiceResult, String> {
        // Never enable arbitrary provider egress. Only the canonical
        // server-owned Echo sender exposes an independent bounded STOP that
        // does not require the engine mutex held during this voice turn.
        if self.remote.is_some()
            || self.interrupt.cancelled.load(Ordering::Acquire)
            || self.interrupt.backend_stop.lock().is_none()
        {
            return unsupported_visitor_voice_egress();
        }
        // Stage the exact PCM under the canonical publication transaction.
        // This is a bounded in-process handoff, not an external sender.
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        self.stage_microphone_audio(&sender, pcm_s16le_mono_16khz)?;
        let pcm_s16le_mono_16khz = receiver
            .try_recv()
            .map_err(|_| "RT1_MICROPHONE_QUEUE_UNAVAILABLE".to_owned())?;
        self.manager.authorize(&self.id)?;
        let callback = Arc::clone(&self.interrupt);
        let result = self
            .engine
            .lock()
            .voice_turn(pcm_s16le_mono_16khz, move |handle| {
                *callback.turn.lock() = Some(handle.clone());
                if callback.cancelled.load(Ordering::Acquire) {
                    let _ = handle.interrupt();
                }
            })
            .map_err(|error| error.code().to_owned());
        // Withdrawing publication wins over a late provider success.
        self.manager.authorize(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        result
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
        // The manager's stop callback deliberately cannot return a network
        // receipt. An explicit close MUST inspect the remote STOP outcome,
        // including when the visitor registration was already revoked.
        // Attempt local cleanup regardless of the remote result.
        let remote_stop = self
            .remote
            .as_ref()
            .map_or(Ok(()), |remote| remote.revoke_confirmed());
        let local_stop = {
            let mut engine = self.engine.lock();
            if engine.session_revocation_handle().is_some() {
                engine
                    .stop_visitor_provider_confirmed()
                    .map_err(|error| error.code().to_owned())
            } else {
                Ok(())
            }
        };
        finish_close(remote_stop, local_stop)
    }
}

impl Drop for Rt1VisitorConversation {
    fn drop(&mut self) {
        let _ = self.manager.close(&self.id);
        self.interrupt.stop();
        // Best-effort retry is never treated as a durable STOP receipt.
        if let Some(remote) = &self.remote {
            let _ = remote.revoke_confirmed();
        }
        let mut engine = self.engine.lock();
        if engine.session_revocation_handle().is_some() {
            let _ = engine.stop_visitor_provider_confirmed();
        }
    }
}

// A remote STOP without both sender receipts must never be reported as a
// successful close, even when local engine cleanup succeeded.
fn finish_close(
    remote_stop: Result<(), String>,
    local_stop: Result<(), String>,
) -> Result<(), String> {
    remote_stop?;
    local_stop
}

#[cfg(test)]
mod close_outcome_tests {
    use super::finish_close;

    #[test]
    fn unconfirmed_remote_stop_overrides_successful_local_cleanup() {
        assert_eq!(
            finish_close(Err("RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".into()), Ok(())),
            Err("RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".into())
        );
    }

    #[test]
    fn local_failure_is_preserved_after_confirmed_remote_stop() {
        assert_eq!(
            finish_close(Ok(()), Err("LOCAL_REVOKE_FAILED".into())),
            Err("LOCAL_REVOKE_FAILED".into())
        );
    }

    #[test]
    fn close_requires_both_remote_and_local_success() {
        assert!(finish_close(Ok(()), Ok(())).is_ok());
    }
}

fn validate_microphone_audio(pcm: &[u8]) -> Result<(), String> {
    // Match the existing RT0 voice input duration and PCM framing limits.
    const MAX_VOICE_BYTES: usize = 16_000 * 2 * 30;
    if pcm.is_empty() || pcm.len() > MAX_VOICE_BYTES || pcm.len() % 2 != 0 {
        return Err("RT1_INVALID_MICROPHONE_AUDIO".into());
    }
    Ok(())
}

#[cfg(test)]
mod microphone_staging_tests {
    use super::validate_microphone_audio;

    #[test]
    fn canonical_microphone_frame_limits_match_rt0() {
        assert!(validate_microphone_audio(&[]).is_err());
        assert!(validate_microphone_audio(&[0_u8]).is_err());
        assert!(validate_microphone_audio(&[0_u8; 2]).is_ok());
        assert!(validate_microphone_audio(&vec![0_u8; 16_000 * 2 * 30]).is_ok());
        assert!(validate_microphone_audio(&vec![0_u8; 16_000 * 2 * 30 + 2]).is_err());
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
