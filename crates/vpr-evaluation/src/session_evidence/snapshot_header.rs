use crate::validate_candidate_sha;

use super::{
    LabSessionAggregateError, LabSessionEvidenceSnapshot, RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE,
    RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA,
};

pub(super) fn validate_snapshot_header(
    snapshot: &LabSessionEvidenceSnapshot,
) -> Result<(), LabSessionAggregateError> {
    if snapshot.schema_version != RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA
        || validate_candidate_sha(&snapshot.candidate_sha).is_err()
        || snapshot.provider_state_sha256.len() != 64
        || !snapshot
            .provider_state_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        || snapshot.scope != RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE
        || snapshot.session_sequence == 0
        || snapshot.session_duration_millis == 0
    {
        return Err(LabSessionAggregateError::InvalidSnapshot);
    }
    Ok(())
}
