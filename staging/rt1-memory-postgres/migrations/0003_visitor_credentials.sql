-- Authenticated, independent stable visitor memory capability (not RT1 publication token).
BEGIN;
CREATE TABLE IF NOT EXISTS vpr_memory.visitor_credentials (
 workspace_id text NOT NULL,
 visitor_id text NOT NULL,
 persona_id text NOT NULL,
 token_hash text NOT NULL UNIQUE,
 created_at timestamptz NOT NULL DEFAULT now(),
 revoked_at timestamptz,
 PRIMARY KEY(workspace_id,visitor_id,persona_id),
 CHECK (length(token_hash)=64)
);
CREATE INDEX IF NOT EXISTS visitor_credentials_active_idx
 ON vpr_memory.visitor_credentials(token_hash) WHERE revoked_at IS NULL;
COMMIT;
