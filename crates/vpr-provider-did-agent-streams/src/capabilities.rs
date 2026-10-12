//! Truthful D-ID capabilities: private Echo STOP is a SERVER operation, not
//! a permission to publish LiveKit did.interrupt from the browser.
use vpr_integration::{RealtimeAvatarCapabilities, RealtimeAvatarCapability};

use crate::DidAgentStreamsAvatar;

impl DidAgentStreamsAvatar {
    pub(crate) fn advertised_capabilities(&self) -> RealtimeAvatarCapabilities {
        let audio_input = if self.echo_backend.is_some() {
            RealtimeAvatarCapability::AudioPcmInput
        } else {
            RealtimeAvatarCapability::AudioUrlInput
        };
        let mut capabilities = vec![RealtimeAvatarCapability::TextInput, audio_input];
        if self.echo_backend.is_some() {
            // The private sender alone may STOP Echo; client_control.interrupt
            // deliberately stays false for every viewer session.
            capabilities.push(RealtimeAvatarCapability::Interrupt);
        }
        RealtimeAvatarCapabilities::new(capabilities)
    }
}
