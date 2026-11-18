//! Memory settings domain actions. Caller supplies an authenticated principal.
//! No media bytes, raw transcripts, browser tokens or provider prompts persisted.
use super::{Fact, MemoryError, MemorySource, StablePrincipal, TextMemory};

pub enum MemoryAction<'a> {
    Consent(bool),
    AddOrCorrect { request_id: &'a str, key: &'a str, value: &'a str, retention_days: i32 },
    Forget { request_id: &'a str, key: &'a str },
    EraseAll,
}

pub enum MemoryOutcome {
    ConsentRevision(i64),
    Updated,
    Erased,
}

impl TextMemory {
    /// Trusted application boundary, not a browser/API authorization function.
    /// The HTTP handler must reauthorize a stable account prior to invoking it.
    pub fn execute_action(&self, principal: &StablePrincipal, action: MemoryAction<'_>)
        -> Result<MemoryOutcome, MemoryError> {
        match action {
            MemoryAction::Consent(enabled) => self.set_consent(principal, enabled)
                .map(MemoryOutcome::ConsentRevision),
            MemoryAction::AddOrCorrect { request_id, key, value, retention_days } => {
                self.remember(principal, request_id, key, value,
                    MemorySource::VisitorExplicit, retention_days)?;
                Ok(MemoryOutcome::Updated)
            },
            MemoryAction::Forget { request_id, key } => {
                self.forget(principal, request_id, key)?;
                Ok(MemoryOutcome::Updated)
            },
            MemoryAction::EraseAll => {
                self.erase_visitor(principal)?;
                Ok(MemoryOutcome::Erased)
            }
        }
    }

    /// Current memory view for the authenticated owner of the visitor scope.
    /// Does not use a search query; bounded at MAX_FACTS_PER_PERSONA.
    pub fn list_facts(&self, p: &StablePrincipal, cursor: Option<&str>, limit: i64)
        -> Result<Vec<Fact>, MemoryError> {
        if !(1..=50).contains(&limit) || cursor.is_some_and(|c| !super::valid_id(c, 96)) {
            return Err(MemoryError::InvalidInput);
        }
        let mut conn = self.conn()?;
        let mut tx = conn.transaction().map_err(super::db)?;
        super::advisory_scope_lock(&mut tx, p)?;
        super::ensure_consent(&mut tx, p)?;
        let rows = tx.query(
            "SELECT fact_key,content,revision FROM vpr_memory.facts \
             WHERE workspace_id=$1 AND visitor_id=$2 AND persona_id=$3 \
               AND expires_at>now() AND ($4::text IS NULL OR fact_key > $4) \
             ORDER BY fact_key LIMIT $5",
            &[&p.workspace_id, &p.visitor_id, &p.persona_id, &cursor, &limit],
        ).map_err(super::db)?;
        let result = rows.into_iter().map(|r| Fact {
            key:r.get(0), content:r.get(1), revision:r.get(2)
        }).collect();
        tx.commit().map_err(super::db)?;
        Ok(result)
    }
}
