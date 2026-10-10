use std::sync::Arc;
use vpr_runtime::RealtimeAvatarStopHandle;
use super::{LabError, LabSessionAudience, OwnerLabEngine};

impl OwnerLabEngine {
    /// Lists configured server-owned TTS routes without disclosing credentials.
    #[must_use]
    pub fn tts_selection(&self) -> serde_json::Value {
        serde_json::json!({
            "active": self.tts_name,
            "alternate": self.tts_fallback_name,
        })
    }

    /// Switches the configured TTS engine for the NEXT turn. The HTTP boundary
    /// must reject changes while voice generation is in flight.
    ///
    /// # Errors
    /// Refuses unknown engines and visitor-initiated configuration changes.
    pub fn select_tts_provider(&mut self, name: &str) -> Result<(), LabError> {
        if self.session_audience == Some(LabSessionAudience::Visitor) {
            return Err(LabError::InvalidState);
        }
        if self.tts_name.as_deref() == Some(name) && self.tts.is_some() {
            return Ok(());
        }
        if self.tts_fallback_name.as_deref() != Some(name) || self.tts_fallback.is_none() {
            return Err(LabError::InvalidInput);
        }
        std::mem::swap(&mut self.tts, &mut self.tts_fallback);
        std::mem::swap(&mut self.tts_name, &mut self.tts_fallback_name);
        Ok(())
    }

    /// Returns an isolated stop-only capability only for a server-owned
    /// `LiveKit` avatar. No arbitrary content publisher is copied to the caller.
    #[must_use]
    pub fn backend_stop_handle(&self) -> Option<RealtimeAvatarStopHandle> {
        self.avatar
            .as_ref()
            .and_then(|handle| handle.backend_stop_handle(Arc::clone(&self.provider)))
    }

    /// Identifies the current canonical session for conditional client teardown.
    /// The sequence is stable across revoke/close and changes on every new start.
    #[must_use]
    pub fn current_session_sequence(&self) -> Option<u64> {
        self.session.as_ref().map(|_| self.session_counter)
    }

}
