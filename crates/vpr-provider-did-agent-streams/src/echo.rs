use std::collections::HashSet;
use std::sync::Mutex;

use vpr_integration::{CancellationProbe, ProviderError};

/// Server-owned D-ID Echo transport. Implementation must synthesize speech
/// through a backend-only TTS path and send PCM/WAV on did.audio-stream with
/// D-ID's `echo_token`. No `echo_token` or speech command may reach browser JS.
///
/// The adapter remains disabled until a concrete backend sender is supplied.
pub trait DidEchoBackend: Send + Sync {
    /// Connects the private Echo sender before the viewer receives the session.
    ///
    /// # Errors
    /// Rejects unverified credentials, unavailable `LiveKit` or cancellation.
    fn open(
        &self,
        session_id: &str,
        session_url: &str,
        echo_token: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError>;

    /// Sends backend-generated speech through the authorized Echo sender only.
    ///
    /// # Errors
    /// Rejects revoked/unavailable sessions, TTS failures or cancellation.
    fn speak(
        &self,
        session_id: &str,
        text: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError>;

    /// Cancels current Echo playback without closing the private LiveKit room.
    /// An implementation must send did.interrupt and drop pending utterances;
    /// it must not publish any new audio as part of STOP.
    ///
    /// # Errors
    /// Defaults to unavailable until the backend can confirm STOP.
    fn interrupt(&self, _session_id: &str) -> Result<(), ProviderError> {
        Err(super::unavailable())
    }

    /// Fences the server-owned Echo audio publisher and closes its connection.
    ///
    /// # Errors
    /// Rejects failures to stop the sender; cleanup must never be assumed.
    fn stop(&self, session_id: &str) -> Result<(), ProviderError>;
}

/// Registrations are server-private; ids are opaque and no publisher grant is
/// copied into `RealtimeAvatarSession` or serialized into `LabSignalBundle`.
#[derive(Default)]
pub(crate) struct DidEchoRegistry {
    sessions: Mutex<HashSet<String>>,
}

impl DidEchoRegistry {
    /// Reserves the vendor identity, opens the private sender, and aborts it
    /// if authority was revoked while the transport was being established.
    /// The cancellation and cleanup are one adapter-level invariant; no
    /// viewer token is exposed until this operation returns successfully.
    pub(crate) fn open_session(
        &self,
        backend: &dyn DidEchoBackend,
        id: &str,
        url: &str,
        echo_token: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError> {
        self.insert(id)?;
        if let Err(error) = backend.open(id, url, echo_token, cancellation) {
            // Implementations must undo partial transport setup on failed open.
            self.remove(id)?;
            return Err(error);
        }
        if cancellation.is_cancelled() {
            let cleanup = backend.stop(id);
            self.remove(id)?;
            // Report failed teardown, never successful revocation.
            return Err(cleanup.err().unwrap_or(super::cancelled()));
        }
        Ok(())
    }

    /// Sends no audio. Only registered private Echo sender sessions may STOP.
    pub(crate) fn interrupt_session(
        &self,
        backend: Option<&dyn DidEchoBackend>,
        id: &str,
    ) -> Result<(), ProviderError> {
        let backend = backend.ok_or_else(super::unavailable)?;
        if !self.contains(id)? {
            return Err(super::unavailable());
        }
        backend.interrupt(id)
    }

    pub(crate) fn insert(&self, id: &str) -> Result<(), ProviderError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| super::invalid_response())?;
        // Every Echo session has its own distinct private publisher grant.
        // Duplicate vendor session identities must never silently re-use one.
        if id.is_empty() || sessions.len() >= 256 || sessions.contains(id) {
            return Err(super::invalid_response());
        }
        sessions.insert(id.to_owned());
        Ok(())
    }

    pub(crate) fn contains(&self, id: &str) -> Result<bool, ProviderError> {
        Ok(self
            .sessions
            .lock()
            .map_err(|_| super::invalid_response())?
            .contains(id))
    }

    pub(crate) fn remove(&self, id: &str) -> Result<(), ProviderError> {
        self.sessions
            .lock()
            .map_err(|_| super::invalid_response())?
            .remove(id);
        Ok(())
    }
}
