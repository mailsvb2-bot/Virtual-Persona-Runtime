//! Memory operations of the canonical visitor conversation.
use super::*;
use crate::MemoryContextPort;

// Canonical visitor memory methods; no separate session runtime.
impl Rt1VisitorConversation {
    /// Canonical persona ID, only after checking publication/session scope.
    pub fn memory_persona_id(&self) -> Result<String, String> {
        self.manager.authorize(&self.id)?;
        Ok(self.memory_persona_id.clone())
    }
    /// Read only the capability binding: the engine remains the single owner.
    pub fn memory_credential_matches(&self, credential: &str) -> bool {
        use sha2::{Digest, Sha256};
        let expected: [u8; 32] = Sha256::digest(credential.as_bytes()).into();
        let guard = self.memory_fingerprint.lock();
        let Some(actual) = guard.as_ref() else {
            return false;
        };
        let mut delta = 0_u8;
        for i in 0..32 {
            delta |= actual[i] ^ expected[i];
        }
        delta == 0
    }
    /// Immediately cancel local future provider admission, then invoke the
    /// existing remote both-track STOP. Never hold a registry lock during I/O.
    pub fn revoke_memory_session(&self) -> Result<(), String> {
        self.interrupt.revoke_authority_only();
        self.close()
    }
    pub fn memory_save_receipt_for(&self, credential: &str) -> Option<String> {
        if self.manager.authorize(&self.id).is_err() {
            return None;
        }
        self.engine.lock().memory_save_receipt_for(credential)
    }
    pub fn authorize_memory_action(&self) -> Result<(), String> {
        self.manager.authorize(&self.id)
    }
    pub fn bind_memory(&self, port: Arc<dyn MemoryContextPort>) -> Result<(), String> {
        self.manager.authorize(&self.id)?;
        // One verified memory identity per live visitor engine. Any attempt
        // to switch personal keys within a session is refused, not rebound.
        let fingerprint = port
            .credential_fingerprint()
            .ok_or_else(|| "RT1_MEMORY_CREDENTIAL_REQUIRED".to_owned())?;
        if let Some(current) = *self.memory_fingerprint.lock() {
            if current != fingerprint {
                return Err("RT1_MEMORY_ALREADY_BOUND".to_owned());
            }
        }
        // STOP must see this credential BEFORE an engine can use it. Never
        // hold memory_fingerprint while waiting on engine.lock() or provider I/O.
        // An unsuccessful first bind keeps a conservative STOP entry (safe).
        {
            let mut binding = self.memory_fingerprint.lock();
            if binding.is_some_and(|current| current != fingerprint) {
                return Err("RT1_MEMORY_ALREADY_BOUND".to_owned());
            }
            *binding = Some(fingerprint);
        }
        self.manager.authorize(&self.id)?;
        self.engine
            .lock()
            .bind_memory_port(port)
            .map_err(|_| "RT1_MEMORY_ALREADY_BOUND".to_owned())?;
        self.manager.authorize(&self.id)
    }
}
