use std::collections::HashSet;
use std::sync::Mutex;

use vpr_integration::{CancellationProbe, ProviderError};

/// Server-owned D-ID Echo transport. Implementation must synthesize speech
/// through a backend-only TTS path and send PCM/WAV on did.audio-stream with
/// D-ID's echo_token. No echo_token or speech command may reach browser JS.
///
/// The adapter remains disabled until a concrete backend sender is supplied.
pub trait DidEchoBackend: Send + Sync {
    fn open(
        &self,
        session_id: &str,
        session_url: &str,
        echo_token: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError>;

    fn speak(
        &self,
        session_id: &str,
        text: &str,
        cancellation: &dyn CancellationProbe,
    ) -> Result<(), ProviderError>;

    fn stop(&self, session_id: &str) -> Result<(), ProviderError>;
}

/// Registrations are server-private; ids are opaque and no publisher grant is
/// copied into RealtimeAvatarSession or serialized into LabSignalBundle.
#[derive(Default)]
pub(crate) struct DidEchoRegistry {
    sessions: Mutex<HashSet<String>>,
}

impl DidEchoRegistry {
    pub(crate) fn insert(&self, id: &str) -> Result<(), ProviderError> {
        self.sessions
            .lock()
            .map_err(|_| super::invalid_response())?
            .insert(id.to_owned());
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
