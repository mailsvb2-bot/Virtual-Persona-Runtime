use super::{LabEvidenceError, LabSessionEvidenceRecorder};
use vpr_evaluation::validate_candidate_sha;

impl LabSessionEvidenceRecorder {
    /// Binds all subsequently exported session evidence to one exact candidate/provider state.
    ///
    /// # Errors
    /// Fails for malformed provenance or after any session has already started.
    pub fn bind_provenance(
        &mut self,
        candidate_sha: &str,
        provider_state_sha256: &str,
    ) -> Result<(), LabEvidenceError> {
        if self.session_sequence.is_some() {
            return Err(LabEvidenceError::InvalidState);
        }
        if validate_candidate_sha(candidate_sha).is_err()
            || provider_state_sha256.len() != 64
            || !provider_state_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(LabEvidenceError::InvalidInput);
        }
        candidate_sha.clone_into(&mut self.candidate_sha);
        provider_state_sha256.clone_into(&mut self.provider_state_sha256);
        Ok(())
    }
}
