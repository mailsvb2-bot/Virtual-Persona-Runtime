-- Scope-consent grant epoch is distinct from mutable fact/context revision.
-- Apply only after schema v4, with writes paused by operator release procedure.
BEGIN;
ALTER TABLE vpr_memory.visitor_consent
    ADD COLUMN IF NOT EXISTS grant_epoch bigint NOT NULL DEFAULT 0;
UPDATE vpr_memory.visitor_consent
    SET grant_epoch=1 WHERE enabled=true AND grant_epoch=0;
ALTER TABLE vpr_memory.write_outbox
    ADD COLUMN IF NOT EXISTS grant_epoch bigint NOT NULL DEFAULT 0;
-- Existing queued notes originate in the currently active grant. Do not
-- recreate queues for disabled/erased scopes.
UPDATE vpr_memory.write_outbox o SET grant_epoch=c.grant_epoch
  FROM vpr_memory.visitor_consent c
 WHERE c.workspace_id=o.workspace_id AND c.visitor_id=o.visitor_id
   AND c.persona_id=o.persona_id AND c.enabled=true AND o.grant_epoch=0;
INSERT INTO vpr_memory.schema_versions(version,checksum)
 VALUES(5,'v18-20261118-consent-grant-epoch-1')
 ON CONFLICT(version) DO NOTHING;
COMMIT;
