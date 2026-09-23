mod evidence;
mod owner_capture;
mod owner_context;
mod provider_credentials;
mod providers;
mod state;

pub use evidence::{
    LabAvSyncEvidenceInput, LabAvSyncReference, LabEvidenceError, LabMediaEvidenceInput,
    LabMediaEvidenceKind, LabSessionEvidenceRecorder, LabSessionEvidenceSnapshot,
    LabTextAttemptEvidence, LabTextAttemptStatus, LabVoiceAttemptEvidence, LabVoiceAttemptStatus,
    ParticipantRole, RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA,
};

pub use owner_capture::{
    OwnerCaptureClaim, OwnerCaptureError, OwnerCaptureQuestion, OwnerCaptureSnapshot,
    Rt0OwnerCapture,
};

pub use owner_context::{ReviewedOwnerClaimSnapshot, ReviewedOwnerContextSnapshot};

pub use provider_credentials::ProviderCredentialProfile;
#[cfg(windows)]
pub use provider_credentials::{
    delete_provider_profile, load_provider_profile, save_provider_profile,
};

pub use state::{
    ConversationReadiness, LabClientCommand, LabClientControl, LabClientEvent, LabClientRoute,
    LabError, LabProviderUsage, LabRealtimeTransport, LabSessionAudience, LabSignalBundle,
    LabStatus, LabTextResult, LabVoiceInput, LabVoicePlaybackRegistry, LabVoiceResult,
    LabVoiceSegment, LabVoiceUsage, OwnerContextState, OwnerLabEngine, OwnerLabStartRequest,
    OwnerLabTurnInput,
};

pub use providers::{ProviderBundle, ProviderDescriptor};

#[cfg(all(test, windows))]
mod windows_restart_script_tests {
    use std::path::PathBuf;
    use std::process::Command;

    #[test]
    fn safe_restart_script_parses_in_windows_powershell() {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/windows-owner-lab-restart.ps1");
        let script = script.to_string_lossy().replace('\'', "''");
        let command = format!(
            "$tokens=$null; $errors=$null; [System.Management.Automation.Language.Parser]::ParseFile('{script}',[ref]$tokens,[ref]$errors) | Out-Null; if($errors.Count -gt 0) {{ $errors | ForEach-Object {{ Write-Error $_.Message }}; exit 1 }}"
        );
        let status = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &command])
            .status()
            .expect("Windows PowerShell must be available on the Windows RT0 operator path");
        assert!(
            status.success(),
            "Owner Lab restart script must parse cleanly"
        );
    }
}
