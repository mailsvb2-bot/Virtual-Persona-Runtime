//! Actual `OwnerLabEngine` bridge for a single RT1 visitor, without public HTTP.
//! Caller MUST supply a distinct engine per visitor, never the owner's engine.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use vpr_runtime::{SessionRevocationHandle, TurnInterruptHandle};

use crate::rt1_visitor_session_manager::VisitorSessionManager;
use crate::state::{
    LabError, LabRealtimeTransport, LabSignalBundle, LabTextResult, LabVoiceResult,
    LabVoiceSegment, OwnerLabEngine, OwnerLabStartRequest,
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
        let started = engine.start_visitor(OwnerLabStartRequest { consent });
        let bundle = match started {
            Ok(bundle) => bundle,
            Err(error) => {
                let _ = manager.close(&id);
                return Err(error.code().to_owned());
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
    pub fn text_turn(&self, text: &str) -> Result<LabTextResult, String> {
        self.manager.compute_buffered(&self.id, || {
            if self.interrupt.cancelled.load(Ordering::Acquire) {
                return Err("visitor session cancelled".into());
            }
            let mut engine = self.engine.lock();
            // Revocation does not take the engine lock; it interrupts the
            // in-flight provider operation through its narrow handle.
            self.manager.authorize(&self.id)?;
            let result = engine.text_turn(text, |turn| {
                *self.interrupt.turn.lock() = Some(turn.clone());
                if self.interrupt.cancelled.load(Ordering::Acquire) {
                    let _ = turn.interrupt();
                }
            });
            *self.interrupt.turn.lock() = None;
            result.map_err(|error| error.code().to_owned())
        })
    }

    /// Run a real microphone turn through STT, LLM, and the existing avatar
    /// provider using this visitor's private engine and interrupt authority.
    ///
    /// # Errors
    /// Fails closed on stale publication, malformed audio, or provider failure.
    pub fn voice_turn(&self, pcm_s16le_mono_16khz: Vec<u8>) -> Result<LabVoiceResult, String> {
        self.manager.compute_buffered(&self.id, || {
            if self.interrupt.cancelled.load(Ordering::Acquire) {
                return Err("visitor session cancelled".into());
            }
            let mut engine = self.engine.lock();
            self.manager.authorize(&self.id)?;
            let result = engine.voice_turn(pcm_s16le_mono_16khz, |turn| {
                *self.interrupt.turn.lock() = Some(turn.clone());
                if self.interrupt.cancelled.load(Ordering::Acquire) {
                    let _ = turn.interrupt();
                }
            });
            *self.interrupt.turn.lock() = None;
            self.manager.authorize(&self.id)?;
            result.map_err(|error| error.code().to_owned())
        })
    }

    /// Run the existing streaming voice pipeline while fencing every
    /// client-visible phrase event against the active visitor publication.
    /// This method does not by itself implement a public audio/video transport.
    ///
    /// # Errors
    /// Denies revoked authority, invalid microphone audio or failed provider output.
    pub fn voice_turn_streaming(
        &self,
        pcm_s16le_mono_16khz: Vec<u8>,
        mut emit_segment: impl FnMut(LabVoiceSegment) -> Result<(), String>,
    ) -> Result<LabVoiceResult, String> {
        self.manager.compute_buffered(&self.id, || {
            if self.interrupt.cancelled.load(Ordering::Acquire) {
                return Err("visitor session cancelled".into());
            }
            let mut engine = self.engine.lock();
            self.manager.authorize(&self.id)?;
            let result = engine.voice_turn_streaming(
                pcm_s16le_mono_16khz,
                |turn| {
                    *self.interrupt.turn.lock() = Some(turn.clone());
                    if self.interrupt.cancelled.load(Ordering::Acquire) {
                        let _ = turn.interrupt();
                    }
                },
                |segment| {
                    if self.interrupt.cancelled.load(Ordering::Acquire)
                        || self.manager.authorize(&self.id).is_err()
                    {
                        return Err(LabError::InvalidState);
                    }
                    emit_segment(segment).map_err(|_| LabError::InvalidState)?;
                    if self.interrupt.cancelled.load(Ordering::Acquire)
                        || self.manager.authorize(&self.id).is_err()
                    {
                        return Err(LabError::InvalidState);
                    }
                    Ok(())
                },
            );
            *self.interrupt.turn.lock() = None;
            self.manager.authorize(&self.id)?;
            result.map_err(|error| error.code().to_owned())
        })
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
