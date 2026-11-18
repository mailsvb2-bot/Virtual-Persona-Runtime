//! Erasure lifecycle, queued-record deletion, metadata-only tombstones.
//! Backup purging must be enforced by operator backup retention and restore hooks.
use super::{
    MemoryError, StablePrincipal, TextMemory, advisory_scope_lock, db, ensure_active_credential,
    hex_sha256,
};
impl TextMemory {
    /// Destroy every text-bearing table in a scope within one transaction.
    /// Preserve an opaque tombstone for backup re-erasure after restore.
    pub fn erase_complete(
        &self,
        p: &StablePrincipal,
        backup_retention_days: i32,
    ) -> Result<(), MemoryError> {
        if !(1..=90).contains(&backup_retention_days) {
            return Err(MemoryError::InvalidInput);
        }
        let mut c = self.conn()?;
        let mut tx = c.transaction().map_err(db)?;
        advisory_scope_lock(&mut tx, p)?;
        ensure_active_credential(&mut tx, p)?;
        let digest = hex_sha256(&format!(
            "{}:{}:{}",
            p.workspace_id, p.visitor_id, p.persona_id
        ));
        tx.execute("UPDATE vpr_memory.visitor_consent SET enabled=false,consent_revision=consent_revision+1,changed_at=now()
          WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3",&p.args()).map_err(db)?;
        for table in [
            "facts",
            "operation_receipts",
            "audit",
            "fact_versions",
            "write_outbox",
            "summaries",
        ] {
            // Table names are fixed program literals, never user input.
            let sql = format!(
                "DELETE FROM vpr_memory.{table} WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3"
            );
            tx.execute(&sql, &p.args()).map_err(db)?;
        }
        tx.execute("INSERT INTO vpr_memory.erasure_tombstones(subject_hash,backup_retire_after)
          VALUES($1,now()+$2::int*interval '1 day')
          ON CONFLICT(subject_hash) DO UPDATE SET requested_at=now(),backup_retire_after=EXCLUDED.backup_retire_after,reconciled_at=NULL",
          &[&digest,&backup_retention_days]).map_err(db)?;
        tx.commit().map_err(db)
    }
    /// Maintenance only, batch-limit controls vacuum pressure.
    pub fn purge_memory_expired(&self, max_rows: i64) -> Result<u64, MemoryError> {
        if !(1..=100).contains(&max_rows) {
            return Err(MemoryError::InvalidInput);
        }
        self.purge_expired_batch(max_rows)
    }
    /// Audit backup-erasure completion only when operator has verified every retained snapshot.
    pub fn mark_backup_reconciled(&self, subject_hash: &str) -> Result<(), MemoryError> {
        if subject_hash.len() != 64 || !subject_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(MemoryError::InvalidInput);
        }
        let mut c = self.conn()?;
        let n=c.execute("UPDATE vpr_memory.erasure_tombstones SET reconciled_at=now() WHERE subject_hash=$1 AND backup_retire_after<=now()",&[&subject_hash]).map_err(db)?;
        if n == 0 {
            return Err(MemoryError::Conflict);
        }
        Ok(())
    }
}
