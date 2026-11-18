-- VPR RT1 text-only long-term visitor memory, operator-applied once after RT1 release gate.
-- No media, raw transcripts, or vector embeddings are stored.
BEGIN;
CREATE SCHEMA IF NOT EXISTS vpr_memory;
CREATE TABLE IF NOT EXISTS vpr_memory.visitor_consent (
 workspace_id text NOT NULL,
 visitor_id text NOT NULL,
 persona_id text NOT NULL,
 enabled boolean NOT NULL DEFAULT false,
 consent_revision bigint NOT NULL DEFAULT 0,
 changed_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(workspace_id,visitor_id,persona_id),
 CHECK (length(workspace_id) BETWEEN 1 AND 128),
 CHECK (length(visitor_id) BETWEEN 1 AND 128),
 CHECK (length(persona_id) BETWEEN 1 AND 128)
);
CREATE TABLE IF NOT EXISTS vpr_memory.facts (
 workspace_id text NOT NULL,
 visitor_id text NOT NULL,
 persona_id text NOT NULL,
 fact_key text NOT NULL,
 content text NOT NULL,
 source text NOT NULL,
 revision bigint NOT NULL DEFAULT 1,
 created_at timestamptz NOT NULL DEFAULT now(),
 updated_at timestamptz NOT NULL DEFAULT now(),
 expires_at timestamptz NOT NULL,
 search_tokens tsvector GENERATED ALWAYS AS (to_tsvector('russian', content)) STORED,
 PRIMARY KEY(workspace_id,visitor_id,persona_id,fact_key),
 FOREIGN KEY(workspace_id,visitor_id,persona_id)
   REFERENCES vpr_memory.visitor_consent(workspace_id,visitor_id,persona_id) ON DELETE CASCADE,
 CHECK (length(fact_key) BETWEEN 1 AND 96),
 CHECK (octet_length(content) BETWEEN 1 AND 2048),
 CHECK (source IN ('visitor_explicit','owner_reviewed')),
 CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS facts_scope_recent_idx ON vpr_memory.facts
  (workspace_id,visitor_id,persona_id,updated_at DESC);
CREATE INDEX IF NOT EXISTS facts_search_idx ON vpr_memory.facts USING GIN (search_tokens);
CREATE INDEX IF NOT EXISTS facts_expires_idx ON vpr_memory.facts (expires_at);
-- GDPR-style deletion does not retain a plaintext record of erased content.
-- Revocation increments a version to invalidate contexts captured before consent changes.
CREATE TABLE IF NOT EXISTS vpr_memory.operation_receipts (
 workspace_id text NOT NULL, visitor_id text NOT NULL, persona_id text NOT NULL,
 operation_id text NOT NULL, operation_type text NOT NULL,
 fact_key text NOT NULL, content_hash text NOT NULL,
 applied_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(workspace_id,visitor_id,persona_id,operation_id),
 CHECK (operation_type IN ('remember','forget'))
);
CREATE INDEX IF NOT EXISTS memory_receipts_aging_idx ON vpr_memory.operation_receipts(applied_at);
-- Deployment MUST create a dedicated restricted DB role; do not GRANT to PUBLIC.
COMMIT;
