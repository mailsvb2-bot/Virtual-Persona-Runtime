use super::{
    LabSessionAggregateError, LabSessionEvidenceSnapshot, RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE,
    RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA,
};

pub(super) fn validate_snapshot_header(
    snapshot: &LabSessionEvidenceSnapshot,
) -> Result<(), LabSessionAggregateError> {
    if snapshot.schema_version != RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA
        || snapshot.scope != RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE
        || snapshot.session_sequence == 0
        || snapshot.session_duration_millis == 0
    {
        return Err(LabSessionAggregateError::InvalidSnapshot);
    }
    Ok(())
}
