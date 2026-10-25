//! Actual OwnerLabEngine bridge for a single RT1 visitor, without public HTTP.
//! Caller MUST supply a distinct engine per visitor, never the owner's engine.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use vpr_runtime::{SessionRevocationHandle, TurnInterruptHandle};

use crate::rt1_visitor_session_manager::VisitorSessionManager;
use crate::state::{LabSignalBundle, LabTextResult, OwnerLabEngine, OwnerLabStartRequest};

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
/// This does not provide network exposure, audio chunks or LiveKit sender STOP.
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
        let interrupt = Arc::new(LiveVisitorInterrupt::default());
        let callback = Arc::clone(&interrupt);
        let id = manager.open(visitor_token, move || callback.stop())?;
        let started = engine.start_visitor(OwnerLabStartRequest { consent });
        let bundle = match started {
            Ok(bundle) => bundle,
            Err(error) => {
                let _ = manager.close(&id);
                return Err(error.code().to_owned());
            }
        };
        // A pause can happen during the remote avatar creation. The callback
        // sets cancelled even before the real runtime handle is available.
        let revoker = engine.session_revocation_handle();
        *interrupt.session.lock() = revoker;
        if interrupt.cancelled.load(Ordering::Acquire) || manager.authorize(&id).is_err() {
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

    /// # Errors
    /// Rejects a missing or already-closed visitor registration.
    pub fn close(&self) -> Result<(), String> {
        self.manager.close(&self.id)?;
        let _ = self.engine.lock().revoke();
        Ok(())
    }
}

impl Drop for Rt1VisitorConversation {
    fn drop(&mut self) {
        let _ = self.manager.close(&self.id);
        self.interrupt.stop();
        let _ = self.engine.lock().revoke();
    }
}
