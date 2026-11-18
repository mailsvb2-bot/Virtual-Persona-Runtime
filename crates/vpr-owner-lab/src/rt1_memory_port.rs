//! Canonical RT1 visitor memory adapter; no second LLM/STT/TTS executor.
use super::{HttpResponse, egress_for_token, error_response};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use vpr_owner_lab::{LabError, MemoryContextPort};
use vpr_rt1_text_memory_postgres::{
    MemoryError, StablePrincipal, TextMemory,
    egress::MemoryEgressFence,
    explicit_memory_intent,
    orchestrator::{MemoryOrchestrator, PreparedMemory},
};

pub(super) struct VisitorMemoryPort {
    db: Arc<TextMemory>,
    principal: StablePrincipal,
    token: String,
    nonce: String,
    last_save: Mutex<Option<String>>,
    egress: Arc<MemoryEgressFence>,
    orchestrator: MemoryOrchestrator,
    prepared: Mutex<Option<PreparedMemory>>,
}
impl VisitorMemoryPort {
    pub(super) fn create(
        db: &Arc<TextMemory>,
        principal: StablePrincipal,
        token: &str,
    ) -> Result<Self, HttpResponse> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)
            .map_err(|_| error_response(503, "RT1_MEMORY_ENTROPY_UNAVAILABLE"))?;
        let egress = egress_for_token(token)?;
        Ok(Self {
            db: Arc::clone(&db),
            principal,
            token: token.to_owned(),
            nonce: hex_bytes(&bytes),
            last_save: Mutex::new(None),
            egress: Arc::clone(&egress),
            orchestrator: MemoryOrchestrator::with_fence(Arc::clone(&db), Arc::clone(&egress)),
            prepared: Mutex::new(None),
        })
    }
}
impl VisitorMemoryPort {
    /// A revoked or rotated credential must invalidate an already-running
    /// engine binding, not merely block new HTTP sessions.
    fn authorize_current_key(&self) -> Result<(), MemoryError> {
        let fresh = self.db.authenticate(&self.token)?;
        if fresh != self.principal {
            return Err(MemoryError::NotConsented);
        }
        Ok(())
    }
}
fn hex_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut out, "{byte:02x}").expect("String formatting is infallible");
    }
    out
}

fn constant_time_equal(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= (usize::from(*a.get(i).unwrap_or(&0))) ^ (usize::from(*b.get(i).unwrap_or(&0)));
    }
    diff == 0
}

impl MemoryContextPort for VisitorMemoryPort {
    fn contextualize_user_input(&self, text: &str) -> Result<String, LabError> {
        self.authorize_current_key()
            .map_err(|_| LabError::ConsentRequired)?;
        *self.prepared.lock().map_err(|_| LabError::Internal)? = None;
        match self.orchestrator.prepare(&self.principal, text) {
            Ok(prepared) => {
                let input = prepared.input.clone();
                *self.prepared.lock().map_err(|_| LabError::Internal)? = Some(prepared);
                Ok(input)
            }
            // DB outage must not authorize stale private context; a plain,
            // no-memory turn remains possible without private data egress.
            Err(_) => Ok(text.to_owned()),
        }
    }
    fn authorize_provider_egress(&self) -> Result<Option<Box<dyn Send>>, LabError> {
        self.authorize_current_key()
            .map_err(|_| LabError::ConsentRequired)?;
        let prepared = self.prepared.lock().map_err(|_| LabError::Internal)?.take();
        let Some(prepared) = prepared else {
            return Ok(None);
        };
        self.db
            .validate_revision(&self.principal, prepared.consent_revision)
            .map_err(|_| LabError::ConsentRequired)?;
        let permit = self
            .egress
            .admit_owned(prepared.consent_revision)
            .ok_or(LabError::ConsentRequired)?;
        Ok(Some(Box::new(permit)))
    }
    fn after_success(&self, text: &str, turn_id: u64) {
        if self.authorize_current_key().is_err() {
            if let Ok(mut last) = self.last_save.lock() {
                *last = Some("not_saved:credential_revoked".to_owned());
            }
            return;
        }
        let Some(fact) = explicit_memory_intent(text) else {
            return;
        };
        let digest = Sha256::digest(fact.as_bytes());
        let key = format!("note-{}", hex_bytes(&digest[..12]));
        let operation = format!("turn-{}-{turn_id}", self.nonce);
        let result = self
            .db
            .enqueue_explicit_fact(&self.principal, &operation, &key, &fact, 365);
        // Never turn an already completed audio/video turn into an error.
        // The visitor can inspect a bounded confirmation instead of relying
        // on a possibly false spoken promise.
        if let Ok(mut last) = self.last_save.lock() {
            *last = Some(match result {
                Ok(_) => format!("queued:{key}"),
                Err(_) => "not_saved".to_owned(),
            });
        }
    }
    fn credential_fingerprint(&self) -> Option<[u8; 32]> {
        Some(Sha256::digest(self.token.as_bytes()).into())
    }
    fn receipt_for_credential(&self, token: &str) -> Option<String> {
        // Constant-time compare prevents receipt disclosure to a different
        // capability within the same RT1 session. Fresh database authorization
        // also invalidates a rotated or revoked credential.
        if !constant_time_equal(self.token.as_bytes(), token.as_bytes())
            || self.authorize_current_key().is_err()
        {
            return None;
        }
        self.last_save.lock().ok().and_then(|s| s.clone())
    }
}
