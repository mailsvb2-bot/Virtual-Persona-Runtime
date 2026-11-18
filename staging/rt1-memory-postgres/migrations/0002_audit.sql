-- Run following 0001 on an isolated licensed test PostgreSQL database.
-- Keep no plaintext content in audit records; never store PCM, video, or transcripts.
BEGIN;
CREATE TABLE IF NOT EXISTS vpr_memory.audit (
  audit_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  workspace_id text NOT NULL,
  visitor_id text NOT NULL,
  persona_id text NOT NULL,
  action text NOT NULL CHECK (action IN ('consent_granted','consent_revoked','remember','forget','erase')),
  fact_key text,
  occurred_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS audit_scope_idx ON vpr_memory.audit
  (workspace_id,visitor_id,persona_id,occurred_at DESC);
-- Scope-bound erase additionally removes audit records. Do not keep content copies.
COMMIT;
