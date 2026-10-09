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
