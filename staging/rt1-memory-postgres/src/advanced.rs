//! Canonical PostgreSQL memory intelligence on the existing TextMemory owner.
//! No additional LLM, second database or media storage.
use super::policy::{Reconcile, candidate, reconcile};
use super::{
    Fact, MemoryError, StablePrincipal, TextMemory, advisory_scope_lock, db, ensure_consent,
    hex_sha256, valid_id,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RichFact {
    pub key: String,
    pub content: String,
    pub revision: i64,
    pub importance: i16,
    pub provenance: String,
    pub valid_from: Option<std::time::SystemTime>,
    pub valid_until: Option<std::time::SystemTime>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsolidationResult {
    Inserted,
    Unchanged,
    Corrected,
    NeedsConfirmation,
}
impl TextMemory {
    /// Save only user-approved notes. Same key corrects; different similar facts require confirmation.
    /// History is captured within the same DB transaction; a conflict never silently overwrites.
    pub fn remember_reviewed(
        &self,
        p: &StablePrincipal,
        operation_id: &str,
        key: &str,
        content: &str,
        days: i32,
    ) -> Result<ConsolidationResult, MemoryError> {
        self.remember_reviewed_in_grant(p, operation_id, key, content, days, None)
    }
    /// A queued note is bound to the exact consent grant in which it was
    /// accepted. Revocation followed by re-consent must never replay the note.
    pub(crate) fn remember_reviewed_in_grant(
        &self,
        p: &StablePrincipal,
        operation_id: &str,
        key: &str,
        content: &str,
        days: i32,
        expected_grant: Option<i64>,
    ) -> Result<ConsolidationResult, MemoryError> {
        let fact = candidate(key, content, days).ok_or(MemoryError::InvalidInput)?;
        if !valid_id(operation_id, 96) {
            return Err(MemoryError::InvalidInput);
        }
        let mut conn = self.conn()?;
        let mut tx = conn.transaction().map_err(db)?;
        advisory_scope_lock(&mut tx, p)?;
        ensure_consent(&mut tx, p)?;
        if let Some(expected) = expected_grant {
            let row=tx.query_one("SELECT grant_epoch FROM vpr_memory.visitor_consent WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND enabled=true", &p.args()).map_err(db)?;
            if row.get::<_, i64>(0) != expected {
                return Err(MemoryError::NotConsented);
            }
            // The worker selects jobs before releasing its read transaction. A
            // concurrent correction/forget may remove the queued job before we
            // acquire this scope lock. The job must STILL EXIST, and must still
            // carry the exact payload/grant that was originally authorized.
            // Check with the mutation under the SAME advisory scope lock.
            let queued_hash = hex_sha256(&format!(
                "{}\0{}\0{}\0{}",
                key, content, "visitor_explicit", days
            ));
            let pending=tx.query_opt(
                "SELECT 1 FROM vpr_memory.write_outbox WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND operation_id=$4 AND fact_key=$5 AND content_hash=$6 AND grant_epoch=$7 AND done_at IS NULL",
                &[&p.workspace_id,&p.visitor_id,&p.persona_id,&operation_id,&key,&queued_hash,&expected],
            ).map_err(db)?;
            if pending.is_none() {
                return Err(MemoryError::NotConsented);
            }
        }
        let old=tx.query_opt("SELECT content,revision FROM vpr_memory.facts WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND fact_key=$4 FOR UPDATE",
            &[&p.workspace_id,&p.visitor_id,&p.persona_id,&key]).map_err(db)?;
        let previous = old.as_ref().map(|r| r.get::<_, String>(0));
        let revision = old.as_ref().map_or(1, |r| r.get::<_, i64>(1) + 1);
        let hash = hex_sha256(&format!("{}:{}:{}", key, content, days));
        let receipt=tx.query_opt("SELECT operation_type,fact_key,content_hash FROM vpr_memory.operation_receipts WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND operation_id=$4",
            &[&p.workspace_id,&p.visitor_id,&p.persona_id,&operation_id]).map_err(db)?;
        if let Some(row) = receipt {
            if row.get::<_, String>(0) == "remember"
                && row.get::<_, String>(1) == key
                && row.get::<_, String>(2) == hash
            {
                tx.commit().map_err(db)?;
                return Ok(ConsolidationResult::Unchanged);
            }
            return Err(MemoryError::Conflict);
        }
        if let Some(previous) = previous.as_ref() {
            if reconcile(key, previous, key, content) == Reconcile::Same {
                // Even an unchanged explicit correction invalidates a queued,
                // older payload for the same key. Otherwise the later worker
                // can overwrite an already-confirmed user choice.
                if expected_grant.is_none() {
                    tx.execute("DELETE FROM vpr_memory.write_outbox WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND fact_key=$4",
                        &[&p.workspace_id,&p.visitor_id,&p.persona_id,&key]).map_err(db)?;
                }
                // Idempotency must be durable even when the content is unchanged:
                // otherwise the same operation id can later mutate a different fact.
                tx.execute("INSERT INTO vpr_memory.operation_receipts(workspace_id,visitor_id,persona_id,operation_id,operation_type,fact_key,content_hash) VALUES($1,$2,$3,$4,'remember',$5,$6)",
                  &[&p.workspace_id,&p.visitor_id,&p.persona_id,&operation_id,&key,&hash]).map_err(db)?;
                tx.commit().map_err(db)?;
                return Ok(ConsolidationResult::Unchanged);
            }
        }
        let similar=tx.query_opt("SELECT fact_key,content FROM vpr_memory.facts WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND fact_key<>$4 AND superseded_by IS NULL AND expires_at>now() AND dedupe_hash=$5 LIMIT 1",
            &[&p.workspace_id,&p.visitor_id,&p.persona_id,&key,&fact.digest]).map_err(db)?;
        // A different key which duplicates an existing fact must not produce a
        // false 'saved' receipt for a key that was never written.
        if similar.is_some() {
            return Err(MemoryError::Conflict);
        }
        let count:i64=tx.query_one("SELECT count(*) FROM vpr_memory.facts WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND expires_at>now()",
            &p.args()).map_err(db)?.get(0);
        if count >= super::MAX_FACTS_PER_PERSONA && old.is_none() {
            return Err(MemoryError::Capacity);
        }
        // An immediate reviewed correction supersedes deferred writes for the
        // same fact key. Workers already holding a stale snapshot will fail
        // their pending-row recheck above instead of restoring old content.
        if expected_grant.is_none() {
            tx.execute("DELETE FROM vpr_memory.write_outbox WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND fact_key=$4",
                &[&p.workspace_id,&p.visitor_id,&p.persona_id,&key]).map_err(db)?;
        }
        tx.execute("INSERT INTO vpr_memory.facts AS f (workspace_id,visitor_id,persona_id,fact_key,content,source,expires_at,importance,provenance,confidence,dedupe_hash)
          VALUES ($1,$2,$3,$4,$5,'visitor_explicit',now()+$6::int*interval '1 day',$7,'visitor_explicit',100,$8)
          ON CONFLICT(workspace_id,visitor_id,persona_id,fact_key) DO UPDATE SET content=EXCLUDED.content,updated_at=now(),expires_at=EXCLUDED.expires_at,
          revision=f.revision+1,importance=EXCLUDED.importance,dedupe_hash=EXCLUDED.dedupe_hash,superseded_by=NULL",
          &[&p.workspace_id,&p.visitor_id,&p.persona_id,&key,&content,&days,&(fact.importance as i16),&fact.digest]).map_err(db)?;
        tx.execute("INSERT INTO vpr_memory.fact_versions(workspace_id,visitor_id,persona_id,fact_key,revision,content,action,provenance)
          VALUES($1,$2,$3,$4,$5,$6,$7,'visitor_explicit')",
          &[&p.workspace_id,&p.visitor_id,&p.persona_id,&key,&revision,&content,
            &if old.is_some(){"corrected"}else{"created"}]).map_err(db)?;
        tx.execute("UPDATE vpr_memory.visitor_consent SET consent_revision=consent_revision+1,changed_at=now() WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3",&p.args()).map_err(db)?;
        tx.execute("INSERT INTO vpr_memory.operation_receipts(workspace_id,visitor_id,persona_id,operation_id,operation_type,fact_key,content_hash)
          VALUES($1,$2,$3,$4,'remember',$5,$6)",
          &[&p.workspace_id,&p.visitor_id,&p.persona_id,&operation_id,&key,&hash]).map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(if old.is_some() {
            ConsolidationResult::Corrected
        } else {
            ConsolidationResult::Inserted
        })
    }
    /// Candidate contradictions are *suggestions*, never automatic cross-key overwrites.
    pub fn possible_contradictions(
        &self,
        p: &StablePrincipal,
        new_text: &str,
    ) -> Result<Vec<Fact>, MemoryError> {
        let new_cat = super::policy::category(new_text);
        if new_cat == "explicit_note" {
            return Ok(vec![]);
        }
        let items = self.list_facts(p, None, 50)?;
        Ok(items
            .into_iter()
            .filter(|f| {
                super::policy::category(&f.content) == new_cat
                    && super::policy::fingerprint(&f.content)
                        != super::policy::fingerprint(new_text)
            })
            .take(5)
            .collect())
    }
    /// Cheap scope epoch for cache validation: consent generation plus the
    /// newest fact mutation timestamp. Both are read under one scope lock.
    /// Caller must not treat this as a provider-egress permit.
    pub fn memory_state_epoch(&self, p: &StablePrincipal) -> Result<(i64, i64), MemoryError> {
        let mut c = self.conn()?;
        let mut tx = c.transaction().map_err(db)?;
        advisory_scope_lock(&mut tx, p)?;
        let revision = ensure_consent(&mut tx, p)?;
        let micros:i64=tx.query_one(
            "SELECT COALESCE(MAX((extract(epoch FROM updated_at)*1000000)::bigint),0) FROM vpr_memory.facts \
             WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND expires_at>now() AND superseded_by IS NULL",
            &p.args()).map_err(db)?.get(0);
        tx.commit().map_err(db)?;
        Ok((revision, micros))
    }
    /// Hybrid retrieval: exact key, FTS rank, importance and freshness; NO irrelevant fallback.
    pub fn recall_hybrid(
        &self,
        p: &StablePrincipal,
        query: &str,
        max_rows: i64,
    ) -> Result<Vec<Fact>, MemoryError> {
        if query.len() > 256 || !(1..=8).contains(&max_rows) || query.contains('\0') {
            return Err(MemoryError::InvalidInput);
        }
        if query.trim().is_empty() {
            return Ok(self.recall(p, "", max_rows)?.facts);
        }
        let terms = super::policy::useful_terms(query).join(" ");
        if terms.is_empty() {
            return Ok(vec![]);
        }
        let mut conn = self.conn()?;
        let mut tx = conn.transaction().map_err(db)?;
        advisory_scope_lock(&mut tx, p)?;
        ensure_consent(&mut tx, p)?;
        let rows=tx.query("WITH q AS (SELECT websearch_to_tsquery('russian',$5) AS phrase)
          SELECT fact_key,content,revision FROM vpr_memory.facts, q
          WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND expires_at>now()
            AND superseded_by IS NULL AND (search_tokens @@ q.phrase OR fact_key=$6)
            AND (valid_from IS NULL OR valid_from<=now()) AND (valid_until IS NULL OR valid_until>now())
          ORDER BY (CASE WHEN fact_key=$6 THEN 10 ELSE 0 END)+ts_rank(search_tokens,q.phrase)*3+importance::real/5 DESC,updated_at DESC LIMIT $4",
          &[&p.workspace_id,&p.visitor_id,&p.persona_id,&max_rows,&terms,&query.trim()]).map_err(db)?;
        let facts = rows
            .into_iter()
            .map(|r| Fact {
                key: r.get(0),
                content: r.get(1),
                revision: r.get(2),
            })
            .collect();
        tx.commit().map_err(db)?;
        Ok(facts)
    }
    /// Explicitly set validity window with canonical consent and identity checks.
    pub fn set_validity(
        &self,
        p: &StablePrincipal,
        key: &str,
        from: Option<i64>,
        until: Option<i64>,
    ) -> Result<(), MemoryError> {
        if !valid_id(key, 96) || from.zip(until).is_some_and(|(a, b)| a >= b) {
            return Err(MemoryError::InvalidInput);
        }
        let mut conn = self.conn()?;
        let mut tx = conn.transaction().map_err(db)?;
        advisory_scope_lock(&mut tx, p)?;
        ensure_consent(&mut tx, p)?;
        let n=tx.execute("UPDATE vpr_memory.facts SET valid_from=to_timestamp($5::bigint::double precision),valid_until=to_timestamp($6::bigint::double precision),updated_at=now() WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 AND fact_key=$4",
          &[&p.workspace_id,&p.visitor_id,&p.persona_id,&key,&from,&until]).map_err(db)?;
        if n != 1 {
            return Err(MemoryError::InvalidInput);
        }
        tx.execute("UPDATE vpr_memory.visitor_consent SET consent_revision=consent_revision+1,changed_at=now() WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3",&p.args()).map_err(db)?;
        tx.commit().map_err(db)
    }
    /// Store bounded, explicitly approved summary; never assume raw transcript consent.
    pub fn save_approved_summary(
        &self,
        p: &StablePrincipal,
        key: &str,
        summary: &str,
        source_digest: &str,
    ) -> Result<(), MemoryError> {
        if !valid_id(key, 96)
            || summary.is_empty()
            || summary.len() > 4096
            || !valid_id(source_digest, 96)
        {
            return Err(MemoryError::InvalidInput);
        }
        let mut conn = self.conn()?;
        let mut tx = conn.transaction().map_err(db)?;
        advisory_scope_lock(&mut tx, p)?;
        ensure_consent(&mut tx, p)?;
        tx.execute("INSERT INTO vpr_memory.summaries AS s(workspace_id,visitor_id,persona_id,summary_key,content,source_digest)
          VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT(workspace_id,visitor_id,persona_id,summary_key)
          DO UPDATE SET content=EXCLUDED.content,source_digest=EXCLUDED.source_digest,updated_at=now()",
          &[&p.workspace_id,&p.visitor_id,&p.persona_id,&key,&summary,&source_digest]).map_err(db)?;
        tx.execute("UPDATE vpr_memory.visitor_consent SET consent_revision=consent_revision+1,changed_at=now() WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3",&p.args()).map_err(db)?;
        tx.commit().map_err(db)
    }
    /// Deterministic, compact summary over ALREADY APPROVED facts only. This
    /// creates neither a second LLM call nor new inferred biographical claims.
    /// The approved summary can be saved separately via save_approved_summary.
    pub fn draft_fact_summary(&self, p: &StablePrincipal) -> Result<(String, String), MemoryError> {
        let mut all = self.list_facts(p, None, 50)?;
        all.truncate(16);
        let mut summary = String::from("Подтверждённые пользователем сведения:\n");
        let mut source = String::new();
        for item in &all {
            let Ok(quoted) = serde_json::to_string(&item.content) else {
                continue;
            };
            let line = format!("- {quoted}\n");
            if summary.len() + line.len() > 3800 {
                break;
            }
            summary.push_str(&line);
            source.push_str(&item.key);
            source.push(':');
            source.push_str(&item.revision.to_string());
            source.push(';');
        }
        Ok((summary, super::hex_sha256(&source)))
    }
    pub fn list_summaries(
        &self,
        p: &StablePrincipal,
    ) -> Result<Vec<(String, String)>, MemoryError> {
        let mut conn = self.conn()?;
        let mut tx = conn.transaction().map_err(db)?;
        advisory_scope_lock(&mut tx, p)?;
        ensure_consent(&mut tx, p)?;
        let rows=tx.query("SELECT summary_key,content FROM vpr_memory.summaries WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 ORDER BY updated_at DESC LIMIT 8",&p.args()).map_err(db)?;
        let items = rows.into_iter().map(|r| (r.get(0), r.get(1))).collect();
        tx.commit().map_err(db)?;
        Ok(items)
    }
}
