//! Canonical RT0/RT1 avatar signaling using the existing runtime turn.
//! Visitor revoke registers cancellation before provider I/O.
use super::*;

impl OwnerLabEngine {
    /// Applies one browser signaling/content operation through a fresh canonical turn.
    ///
    /// # Errors
    /// Returns a stable fail-closed reason if session/authority/provider state is not current.
    pub fn apply(&mut self, input: OwnerLabTurnInput) -> Result<(), LabError> {
        self.apply_with_interrupt(input, |_| {})
    }
    /// Register a cancellable canonical signaling turn before provider I/O.
    /// # Errors
    /// Denies stale sessions or provider failures.
    pub fn apply_with_interrupt(
        &mut self,
        input: OwnerLabTurnInput,
        register_interrupt: impl FnOnce(TurnInterruptHandle),
    ) -> Result<(), LabError> {
        if self.session_audience == Some(LabSessionAudience::Visitor)
            && matches!(
                input,
                OwnerLabTurnInput::Text(_) | OwnerLabTurnInput::AudioUrl(_)
            )
        {
            return Err(LabError::Runtime(Rt0ReasonCode::AuthScopeDenied));
        }
        let turn = self.new_turn()?;
        register_interrupt(turn.interrupt_handle());
        let handle = self.avatar.as_ref().ok_or(LabError::InvalidState)?;
        match input {
            OwnerLabTurnInput::Answer(answer) => {
                if answer.kind != "answer" || answer.sdp.trim().is_empty() {
                    return Err(LabError::InvalidInput);
                }
                turn.submit_realtime_avatar_answer(self.provider.as_ref(), handle, &answer)
            }
            OwnerLabTurnInput::Ice(candidate) => {
                turn.submit_realtime_avatar_ice(self.provider.as_ref(), handle, &candidate)
            }
            OwnerLabTurnInput::Text(text) => {
                if text.trim().is_empty() {
                    return Err(LabError::InvalidInput);
                }
                if let Some(tts) = self.tts.as_deref() {
                    turn.speak_realtime_avatar_text_with_tts(
                        tts,
                        self.provider.as_ref(),
                        handle,
                        &text,
                    )
                } else {
                    turn.speak_realtime_avatar_text(self.provider.as_ref(), handle, &text)
                }
            }
            OwnerLabTurnInput::AudioUrl(url) => {
                if url.trim().is_empty() {
                    return Err(LabError::InvalidInput);
                }
                turn.speak_realtime_avatar_audio_url(self.provider.as_ref(), handle, &url)
            }
            OwnerLabTurnInput::Interrupt => {
                turn.interrupt_realtime_avatar(self.provider.as_ref(), handle)
            }
        }
        .map_err(map_provider_execution)
    }
}
