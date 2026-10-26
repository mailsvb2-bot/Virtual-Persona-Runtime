//! Actual `OwnerLabEngine` bridge for a single RT1 visitor, without public HTTP.
//! Caller MUST supply a distinct engine per visitor, never the owner's engine.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use vpr_runtime::{SessionRevocationHandle, TurnInterruptHandle};

use crate::rt1_visitor_session_manager::VisitorSessionManager;
use crate::state::{
    LabSignalBundle, LabTextResult, OwnerLabEngine,
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
        // Provider startup currently has no enforceable deadline or remote
        // sender STOP. Holding the canonical workspace transaction across this
        // unbounded call prevents owner pause/unpublish from committing. A
        // detached startup is also unsafe: it can create remote media after a
        // revoke. Reject before creating any provider resource until the
        // provider contract supports bounded cancel + confirmed teardown.
        let _ = consent;
        let _ = manager.close(&id);
        Err("RT1_PROVIDER_STARTUP_REQUIRES_BOUNDED_CANCEL_AND_STOP".into())
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

#[cfg(test)]
mod rt1_interrupt_concurrency_tests {
    use super::*;
    use std::sync::mpsc::{channel, sync_channel};
    use std::time::Duration;

    #[test]
    fn visitor_stop_does_not_wait_for_a_hung_provider_worker() {
        // A remote provider may be stuck in startup. The revocation callback
        // must not acquire the engine mutex or wait for that remote request.
        let interrupt = Arc::new(LiveVisitorInterrupt::default());
        let (started_tx, started_rx) = sync_channel(1);
        let (release_tx, release_rx) = channel::<()>();
        let provider = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();

        let stopped = Arc::clone(&interrupt);
        let (done_tx, done_rx) = sync_channel(1);
        let revoker = std::thread::spawn(move || {
            stopped.stop();
            done_tx.send(()).unwrap();
        });
        let result = done_rx.recv_timeout(Duration::from_secs(2));
        release_tx.send(()).unwrap();
        provider.join().unwrap();
        revoker.join().unwrap();
        assert!(result.is_ok(), "visitor local STOP waited for provider I/O");
        assert!(interrupt.cancelled.load(Ordering::Acquire));
    }
}
