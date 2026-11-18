//! Bounded retry policy and maintenance tick. No network retries in the real-time turn.
use super::{MemoryError, StablePrincipal, TextMemory};
use std::time::Duration;
pub struct MaintenanceReport {
    pub expired: u64,
    pub outbox_completed: u64,
}
pub fn retry_delay(attempts: u32) -> Duration {
    Duration::from_millis((100_u64.saturating_mul(1_u64 << attempts.min(6))).min(5000))
}
impl TextMemory {
    pub fn maintenance_tick(
        &self,
        principal: &StablePrincipal,
    ) -> Result<MaintenanceReport, MemoryError> {
        self.check_schema()?;
        let expired = self.purge_memory_expired(100)?;
        let completed = self.drain_scope_outbox(principal, 8)?;
        Ok(MaintenanceReport {
            expired,
            outbox_completed: completed,
        })
    }
}

/// Scan outstanding writes after process restart and periodically while live.
/// Only active credentials/consents qualify. No token plaintext is retained;
/// authorization is rechecked in each transactional write.
impl TextMemory {
    pub fn drain_due_outbox(&self, max_scopes: i64) -> Result<u64, MemoryError> {
        if !(1..=16).contains(&max_scopes) {
            return Err(MemoryError::InvalidInput);
        }
        let mut c = self.conn()?;
        let rows=c.query("SELECT DISTINCT o.workspace_id,o.visitor_id,o.persona_id,k.token_hash  FROM vpr_memory.write_outbox o JOIN vpr_memory.visitor_credentials k    ON k.workspace_id=o.workspace_id AND k.visitor_id=o.visitor_id AND k.persona_id=o.persona_id  JOIN vpr_memory.visitor_consent s ON s.workspace_id=o.workspace_id    AND s.visitor_id=o.visitor_id AND s.persona_id=o.persona_id  WHERE o.done_at IS NULL AND o.next_attempt_at<=now()    AND k.revoked_at IS NULL AND s.enabled=true  ORDER BY o.workspace_id,o.visitor_id,o.persona_id,k.token_hash LIMIT $1", &[&max_scopes]).map_err(super::db)?;
        let principals: Vec<_> = rows
            .into_iter()
            .map(|r| {
                let (w, v, p, h): (String, String, String, String) =
                    (r.get(0), r.get(1), r.get(2), r.get(3));
                super::StablePrincipal::from_verified_identity(&w, &v, &p, &h)
            })
            .collect();
        drop(c);
        let mut count = 0;
        for principal in principals.into_iter().flatten() {
            count += self.drain_scope_outbox(&principal, 8)?;
        }
        Ok(count)
    }
}
