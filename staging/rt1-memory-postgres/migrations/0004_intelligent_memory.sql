-- Additive, text-only memory intelligence. Apply under operator-controlled migration.
BEGIN;
CREATE TABLE IF NOT EXISTS vpr_memory.schema_versions (
  version integer PRIMARY KEY, checksum text NOT NULL, applied_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE vpr_memory.facts
  ADD COLUMN IF NOT EXISTS importance smallint NOT NULL DEFAULT 1,
  ADD COLUMN IF NOT EXISTS valid_from timestamptz,
  ADD COLUMN IF NOT EXISTS valid_until timestamptz,
  ADD COLUMN IF NOT EXISTS provenance text NOT NULL DEFAULT 'visitor_explicit',
  ADD COLUMN IF NOT EXISTS confidence smallint NOT NULL DEFAULT 100,
  ADD COLUMN IF NOT EXISTS dedupe_hash text,
  ADD COLUMN IF NOT EXISTS superseded_by text;
ALTER TABLE vpr_memory.facts ADD CONSTRAINT vpr_memory_importance_range CHECK (importance BETWEEN 1 AND 5);
ALTER TABLE vpr_memory.facts ADD CONSTRAINT vpr_memory_confidence_range CHECK (confidence BETWEEN 0 AND 100);
CREATE INDEX IF NOT EXISTS facts_active_rank_idx ON vpr_memory.facts
  (workspace_id,visitor_id,persona_id,importance DESC,updated_at DESC)
  WHERE superseded_by IS NULL;
CREATE INDEX IF NOT EXISTS facts_dedupe_idx ON vpr_memory.facts
  (workspace_id,visitor_id,persona_id,dedupe_hash);
CREATE TABLE IF NOT EXISTS vpr_memory.fact_versions (
  workspace_id text NOT NULL, visitor_id text NOT NULL, persona_id text NOT NULL,
  fact_key text NOT NULL, revision bigint NOT NULL, content text NOT NULL,
  action text NOT NULL CHECK (action IN ('created','corrected','superseded','expired')),
  provenance text NOT NULL,
  recorded_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY(workspace_id,visitor_id,persona_id,fact_key,revision),
  CHECK (octet_length(content) BETWEEN 1 AND 2048)
);
CREATE TABLE IF NOT EXISTS vpr_memory.write_outbox (
  workspace_id text NOT NULL, visitor_id text NOT NULL, persona_id text NOT NULL,
  operation_id text NOT NULL, fact_key text NOT NULL,
  content text NOT NULL, content_hash text NOT NULL,
  retention_days integer NOT NULL CHECK(retention_days BETWEEN 1 AND 3650),
  created_at timestamptz NOT NULL DEFAULT now(),
  claimed_at timestamptz, attempts integer NOT NULL DEFAULT 0,
  next_attempt_at timestamptz NOT NULL DEFAULT now(),
  done_at timestamptz,
  consent_revision bigint NOT NULL,
  PRIMARY KEY(workspace_id,visitor_id,persona_id,operation_id),
  CHECK (octet_length(content) BETWEEN 1 AND 2048)
);
CREATE INDEX IF NOT EXISTS memory_outbox_due_idx ON vpr_memory.write_outbox(next_attempt_at,created_at)
 WHERE done_at IS NULL;
CREATE TABLE IF NOT EXISTS vpr_memory.summaries (
  workspace_id text NOT NULL, visitor_id text NOT NULL, persona_id text NOT NULL,
  summary_key text NOT NULL, content text NOT NULL, source_digest text NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY(workspace_id,visitor_id,persona_id,summary_key),
  CHECK (octet_length(content) BETWEEN 1 AND 4096)
);
CREATE TABLE IF NOT EXISTS vpr_memory.erasure_tombstones (
  -- No content or credential secrets, only opaque hashes for backup reconciliation.
  subject_hash text PRIMARY KEY, requested_at timestamptz NOT NULL DEFAULT now(),
  backup_retire_after timestamptz NOT NULL, reconciled_at timestamptz
);
CREATE TABLE IF NOT EXISTS vpr_memory.memory_metrics (
  metric_day date NOT NULL DEFAULT CURRENT_DATE,
  metric text NOT NULL, count bigint NOT NULL DEFAULT 0,
  PRIMARY KEY(metric_day,metric),
  CHECK(metric ~ '^[a-z_]{1,64}$')
);
-- Explicit deletes remove history/outbox/summaries; no raw input transcript storage.
CREATE INDEX IF NOT EXISTS fact_versions_subject_idx ON vpr_memory.fact_versions
 (workspace_id,visitor_id,persona_id,recorded_at DESC);
INSERT INTO vpr_memory.schema_versions(version,checksum) VALUES (4,'v15-20261118-memory-intelligence-1')
  ON CONFLICT (version) DO NOTHING;
COMMIT;
