//! Low-cost optional memory boundary of the EXISTING OwnerLabEngine.
//! This source is installed into crates/vpr-owner-lab/src/memory_port.rs.
//! It does not depend on any SQL driver or create a second runtime.
use std::sync::Arc;
use crate::state::{LabError, OwnerLabEngine};

pub trait MemoryContextPort: Send + Sync {
    /// Return the current message with a bounded memory DATA attachment, not
    /// replacement system instructions. No other LLM or conversation path.
    fn contextualize_user_input(&self, text: &str) -> Result<String, LabError>;
    /// A live authorization lease covering provider LLM dispatch.
    /// The executor must keep this RAII value alive until provider operation ends.
    fn authorize_provider_egress(&self) -> Result<Option<Box<dyn Send>>, LabError> {
        Ok(None)
    }
    /// Called only after the canonical turn completed. Never store media.
    fn after_success(&self, text: &str, turn_id: u64);
    /// A receipt can be read only with the credential of the bound memory.
    /// An arbitrary personal key in the same published session is insufficient.
    fn receipt_for_credential(&self, _token: &str) -> Option<String> { None }
    /// Only for cancelling ALL live bindings of the same revoked credential.
    /// Must not authenticate the browser: DB authentication remains canonical.
    fn credential_fingerprint(&self) -> Option<[u8; 32]> { None }
}

impl OwnerLabEngine {
    /// Server-owned reviewed Persona ID, never a browser-submitted ID.
    pub fn memory_persona_id(&self) -> Result<String, LabError> {
        self.reviewed_persona_binding()
            .map(|(id, _)| id.to_owned())
            .ok_or(LabError::InvalidState)
    }
    /// Only a trusted, identity-authenticated visitor factory may call this.
    /// OwnerLabEngine itself remains the sole STT->LLM->TTS->avatar executor.
    pub fn bind_memory_port(&mut self, port: Arc<dyn MemoryContextPort>) -> Result<(), LabError> {
        // Authorized RT1 conversation adapter serializes this operation with
        // provider turns under its existing engine lock. Rebinding after a
        // credential rotation must replace the old, now-revoked port.
        // A live conversation is bound to ONE visitor identity. Attaching a
        // different key would permit mixed-persona context and receipts.
        // A rotated key must start a new authorized RT1 conversation.
        if let Some(current) = &self.memory_port {
            let previous = current.credential_fingerprint();
            let next = port.credential_fingerprint();
            if previous.is_none() || previous != next {
                return Err(LabError::InvalidState);
            }
        }
        self.memory_port = Some(port);
        Ok(())
    }
    pub(crate) fn memory_instructions(&self, text: &str) -> Result<String, LabError> {
        let mut canonical = self.conversation_instructions()?;
        // Spoken/text generation finishes before the database confirms an
        // explicit note. Never claim successful persistence in synthesized
        // speech. The visitor must rely on the separate durable receipt.
        let normalized = text.trim().to_lowercase();
        if normalized.starts_with("запомни:") || normalized.starts_with("remember:") {
            canonical.push_str("\nIf the visitor asks to remember a note, do NOT claim it was saved. Explain briefly that they can verify the note in the Memory panel. Never infer or auto-save other facts.");
        }
        Ok(canonical)
    }
    pub(crate) fn memory_user_input(&self, text: &str) -> Result<String, LabError> {
        match &self.memory_port {
            Some(port) => port.contextualize_user_input(text),
            None => Ok(text.to_owned()),
        }
    }
    pub fn memory_binding_fingerprint(&self) -> Option<[u8; 32]> {
        self.memory_port.as_ref().and_then(|p| p.credential_fingerprint())
    }
    pub(crate) fn memory_save_receipt_for(&self, credential: &str) -> Option<String> {
        self.memory_port.as_ref().and_then(|port| port.receipt_for_credential(credential))
    }
    pub(crate) fn memory_provider_guard(&self) -> Result<Option<Box<dyn Send>>, LabError> {
        match &self.memory_port {
            Some(port) => port.authorize_provider_egress(),
            None => Ok(None),
        }
    }
    pub(crate) fn memory_after_success(&self, text: &str, turn_id: u64) {
        if let Some(port) = &self.memory_port { port.after_success(text, turn_id); }
    }
}
