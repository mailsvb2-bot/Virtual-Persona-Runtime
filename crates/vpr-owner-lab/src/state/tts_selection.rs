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

}
