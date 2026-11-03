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
    LabSignalBundle, LabVoiceResult, LabVoiceSegment, OwnerLabEngine, OwnerLabStartRequest,
    OwnerLabTurnInput,
};

#[path = "rt1_visitor_conversation_helpers.rs"]
mod helpers;
#[path = "rt1_visitor_text_egress.rs"]
mod text_egress;
#[path = "rt1_visitor_voice_stream_staging.rs"]
mod voice_stream_staging;
use helpers::{
    diagnostic_operation_id, finish_close, revoked_diagnostic_offer_error,
    validate_microphone_audio,
};

#[derive(Default)]
struct LiveVisitorInterrupt {
    cancelled: AtomicBool,
    session: Mutex<Option<SessionRevocationHandle>>,
    turn: Mutex<Option<TurnInterruptHandle>>,
    backend_stop: Mutex<Option<RealtimeAvatarStopHandle>>,
}

impl LiveVisitorInterrupt {
    /// Synchronously invalidate runtime provider permits without waiting
    /// for a stalled provider or the conversation engine mutex.
    fn revoke_authority_only(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(turn) = self.turn.lock().clone() {
            let _ = turn.interrupt();
        }
        if let Some(session) = self.session.lock().clone() {
            let _ = session.revoke_authority();
        }
    }

    fn provider_quiescent(&self, timeout: Duration) -> bool {
        self.session
            .lock()
            .as_ref()
            .is_none_or(|session| session.await_provider_quiescence(timeout))
    }

    fn stop(&self) {
        self.revoke_authority_only();
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

/// Keeps the canonical visitor provider-egress lease alive through the HTTP
/// response write. Dropping it signals LOCAL delivery quiescence only.
pub struct Rt1BrowserDeliveryPermit {
    _lease: crate::rt1_visitor_session_manager::VisitorEgressPermit,
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
        let fast_revoke = Arc::clone(&interrupt);
        let on_revoke = Arc::clone(&interrupt);
        let id = manager.open_with_fast_revoke(
            visitor_token,
            move || fast_revoke.revoke_authority_only(),
            move || on_revoke.stop(),
        )?;
        if let Err(error) = manager.authorize_persona(&id, &persona_id, persona_version) {
            let _ = manager.close(&id);
            return Err(error);
        }

        // Register the startup turn BEFORE provider CREATE. A pause during a
        // slow D-ID/LiveKit allocation can now cancel the canonical transport
        // without waiting for the engine mutex or a provider-generated ID.
        let startup_interrupt = Arc::clone(&interrupt);
        let started =
            engine.start_visitor_with_interrupt(OwnerLabStartRequest { consent }, move |handle| {
                *startup_interrupt.turn.lock() = Some(handle.clone());
                if startup_interrupt.cancelled.load(Ordering::Acquire) {
                    let _ = handle.interrupt();
                }
            });
        *interrupt.turn.lock() = None;
        let bundle = match started {
            Ok(bundle) => bundle,
            Err(error) => {
                // A failed remote CREATE can still have allocated an avatar.
                // Never treat manager.close() as proof that the remote sender
                // stopped; explicitly require the canonical provider receipt.
                // A provider error can precede establishment of the visitor
                // session, so STOP may not yet have an addressable target.
                // Report the cleanup uncertainty rather than claim a receipt.
                // The canonical engine can fail after creating a provider
                // session. If a real stop-only handle exists, failure to close
                // it must be surfaced as unconfirmed remote shutdown.
                let has_sender = engine.visitor_avatar_stop_handle().is_some();
                let cleanup = if has_sender {
                    Some(engine.stop_visitor_provider_confirmed())
                } else {
                    None
                };
                let _ = manager.close(&id);
                if matches!(cleanup, Some(Err(_))) {
                    return Err("RT1_PROVIDER_START_FAILED_STOP_UNCONFIRMED".into());
                }
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
        let operation_id = diagnostic_operation_id()?;
        let remote = Arc::new(Rt1RemoteMediaRevocation::new(
            client.clone(),
            operation_id.clone(),
            1,
        )?);
        let interrupt = Arc::new(LiveVisitorInterrupt::default());
        let fast_interrupt = Arc::clone(&interrupt);
        let stop_interrupt = Arc::clone(&interrupt);
        let stop_remote = Arc::clone(&remote);
        let id = manager.open_with_fast_revoke(
            visitor_token,
            move || fast_interrupt.revoke_authority_only(),
            move || {
                stop_interrupt.stop();
                let _ = stop_remote.revoke_confirmed();
            },
        )?;

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
            // CREATE may finish after revoke has already received a STOP
            // receipt. Correlate the actual returned sender before allowing
            // any SDP/voice publication or accepting remote shutdown.
            remote.record_created_session(&session_id)?;
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
        // CREATE/OFFER failure may leave a remote sender running. Treat an
        // unconfirmed compensating STOP as the dominant error.
        let bundle = started.map_err(|error| {
            let _ = manager.close(&id);
            if remote.revoke_confirmed().is_err() {
                "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED".to_owned()
            } else {
                error
            }
        })?;
        // The owner may revoke while the offer is being constructed. Never
        // return a stale SDP bundle without one final publication check.
        if manager
            .authorize_persona(&id, &persona_id, persona_version)
            .is_err()
            || remote.is_revoked()
            || interrupt.cancelled.load(Ordering::Acquire)
        {
            return Err(revoked_diagnostic_offer_error(&manager, &id, &remote));
        }
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

    /// Acquire a final browser-delivery lease using the SAME publication
    /// admission gate as the canonical STT/LLM/TTS provider turns. The lease
    /// must live until the HTTP write finishes, not merely until JSON is built.
    ///
    /// A concurrent owner pause invalidates authority immediately; it must
    /// never claim confirmed sender shutdown while a delivery lease is held.
    /// This is a LOCAL quiescence fence, NOT remote audio/video STOP evidence.
    /// A stalled browser write may still outlive revocation; #319 requires a
    /// bounded cancellable network writer before this is a release gate.
    ///
    /// # Errors
    /// Denies revoked publication, concurrent provider turns, or stale permits.
    pub fn begin_browser_delivery(&self) -> Result<Rt1BrowserDeliveryPermit, String> {
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        let lease = self.manager.begin_provider_egress(&self.id)?;
        if !self.is_active() {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        Ok(Rt1BrowserDeliveryPermit { _lease: lease })
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
        // Same revocation-serialized admission as visitor text and the
        // canonical runtime's provider execution gate. The local queue is
        // never mistaken for permission to transmit microphone PCM.
        let _egress = self.manager.begin_provider_egress(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
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
        *self.interrupt.turn.lock() = None;
        // Withdrawing publication wins over a late provider success, even
        // when the provider returned an error before this final check.
        self.manager.authorize(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        result
    }

    /// Interrupt the canonical visitor turn and its independent private Echo
    /// audio sender. This is NOT a remote video RTP STOP receipt.
    ///
    /// # Errors
    /// Fails closed if private audio STOP is uncertain.
    pub fn interrupt_turn(&self) -> Result<(), String> {
        self.manager.authorize(&self.id)?;
        if self.remote.is_some() || self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_INTERRUPT_DENIED".into());
        }
        let stop = self
            .interrupt
            .backend_stop
            .lock()
            .clone()
            .ok_or_else(|| "RT1_VISITOR_INTERRUPT_DENIED".to_owned())?;
        // Never wait for engine.lock(): a blocked STT/LLM must be interruptible.
        if let Some(turn) = self.interrupt.turn.lock().clone() {
            let _ = turn.interrupt();
        }
        if stop.stop().is_err() {
            // An unconfirmed sender must not accept another voice turn.
            self.interrupt.stop();
            let _ = self.manager.close(&self.id);
            return Err("RT1_VISITOR_INTERRUPT_STOP_UNCONFIRMED".into());
        }
        // A successful audio STOP is not proof that an in-flight provider
        // request has drained. Keep this visitor quarantined if the cancelled
        // transport refuses to return within the bounded wait.
        if !self
            .interrupt
            .provider_quiescent(Duration::from_millis(250))
            || !self
                .manager
                .await_provider_quiescence(&self.id, Duration::from_millis(250))
        {
            self.interrupt.stop();
            let _ = self.manager.close(&self.id);
            return Err("RT1_VISITOR_INTERRUPT_STOP_UNCONFIRMED".into());
        }
        self.manager.authorize(&self.id)?;
        if self.interrupt.cancelled.load(Ordering::Acquire) {
            return Err("RT1_VISITOR_REVOKED".into());
        }
        Ok(())
    }

    /// Streaming is fail-closed: pre/post checks cannot protect egress callbacks.
    /// Buffered and streaming voice share the unsupported egress boundary.
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

    /// True only for the durable worker path that verifies both remote RTP
    /// sender tracks stopped. Canonical D-ID `LiveKit` Echo close is a local
    /// publisher disconnect, not authenticated remote video termination.
    #[must_use]
    pub fn has_remote_video_stop_receipt_path(&self) -> bool {
        self.remote.is_some()
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
        let local_stop = if self
            .interrupt
            .provider_quiescent(Duration::from_millis(250))
        {
            let mut engine = self.engine.lock();
            if engine.session_revocation_handle().is_some() {
                engine
                    .stop_visitor_provider_confirmed()
                    .map_err(|error| error.code().to_owned())
            } else {
                Ok(())
            }
        } else {
            // Do not wait on an engine mutex held by a stalled provider and
            // do not claim confirmed close. The HTTP registry retains this
            // visitor for a subsequent cleanup attempt.
            Err("RT1_PROVIDER_EGRESS_NOT_QUIESCENT".into())
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

fn unsupported_visitor_voice_egress<T>() -> Result<T, String> {
    Err("RT1_VOICE_PROVIDER_EGRESS_NOT_AUTHORIZED".into())
}

#[cfg(test)]
#[path = "rt1_visitor_conversation_tests.rs"]
mod tests;
