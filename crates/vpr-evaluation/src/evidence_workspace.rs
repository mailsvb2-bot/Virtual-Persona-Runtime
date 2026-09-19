use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub const RT0_RELEASE_SPEC_BYTES: &[u8] =
    include_bytes!("../../../docs/releases/RT0_RELEASE_SPEC.md");

pub const RT0_EVIDENCE_REQUIRED_FILES: &[&str] = &[
    "provider-state.json",
    "provider-probe.json",
    "conversation-attempt.json",
    "bound-session-aggregate.json",
    "bound-golden-report.json",
    "private-golden-evidence.json",
    "exit-evidence.json",
    "release-spec.md",
    "ci-evidence.json",
    "e2e-evidence.json",
    "owner-conversation.json",
    "visitor-conversation.json",
    "acceptance.json",
    "quality.json",
    "cost.json",
    "privacy-permissions.json",
    "human-evaluation.json",
    "known-limitations.md",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rt0EvidenceWorkspaceError {
    CreateDirectoryFailed,
    ReleaseSpecConflict,
    ReleaseSpecReadFailed,
    ReleaseSpecWriteFailed,
}

/// Ensures the RT0 evidence directory contains the exact ReleaseSpec bytes compiled with this
/// candidate. Existing matching bytes are left untouched; conflicting bytes fail closed.
///
/// The function deliberately creates no placeholder evidence for provider calls, browser sessions,
/// Golden observations, privacy review, human review, cost, quality, or acceptance.
///
/// # Errors
/// Returns a stable error when the workspace cannot be created/read/written or already contains a
/// different ReleaseSpec.
pub fn prepare_rt0_evidence_workspace(root: &Path) -> Result<bool, Rt0EvidenceWorkspaceError> {
    fs::create_dir_all(root).map_err(|_| Rt0EvidenceWorkspaceError::CreateDirectoryFailed)?;
    let release_spec = root.join("release-spec.md");
    match fs::read(&release_spec) {
        Ok(existing) => {
            if existing == RT0_RELEASE_SPEC_BYTES {
                Ok(false)
            } else {
                Err(Rt0EvidenceWorkspaceError::ReleaseSpecConflict)
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&release_spec)
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        Rt0EvidenceWorkspaceError::ReleaseSpecConflict
                    } else {
                        Rt0EvidenceWorkspaceError::ReleaseSpecWriteFailed
                    }
                })?;
            file.write_all(RT0_RELEASE_SPEC_BYTES)
                .map_err(|_| Rt0EvidenceWorkspaceError::ReleaseSpecWriteFailed)?;
            file.sync_all()
                .map_err(|_| Rt0EvidenceWorkspaceError::ReleaseSpecWriteFailed)?;
            Ok(true)
        }
        Err(_) => Err(Rt0EvidenceWorkspaceError::ReleaseSpecReadFailed),
    }
}
