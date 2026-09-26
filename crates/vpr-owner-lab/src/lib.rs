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
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    fn script_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/windows-owner-lab-restart.ps1")
    }

    #[test]
    fn safe_restart_script_parses_in_windows_powershell() {
        let script = script_path();
        let escaped = script.to_string_lossy().replace('\'', "''");
        let command = format!(
            "$tokens=$null; $errors=$null; [System.Management.Automation.Language.Parser]::ParseFile('{escaped}',[ref]$tokens,[ref]$errors) | Out-Null; if($errors.Count -gt 0) {{ $errors | ForEach-Object {{ Write-Error $_.Message }}; exit 1 }}"
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

    #[test]
    fn restart_script_executes_legacy_migration_self_test() {
        let script = script_path();
        let status = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                script.to_string_lossy().as_ref(),
                "-MigrationSelfTest",
            ])
            .status()
            .expect("Windows PowerShell must execute the migration self-test");
        assert!(
            status.success(),
            "legacy reviewed Persona migration self-test must pass"
        );
    }

    #[test]
    fn restart_script_pins_port_and_egress_for_the_launched_binary() {
        let script = fs::read_to_string(script_path()).expect("restart script must be readable");
        assert!(script.contains("VPR_OWNER_LAB_ALLOW_EGRESS=true"));
        assert!(script.contains("VPR_OWNER_LAB_PORT=$Port"));
        assert!(script.contains("--allow-egress"));
        assert!(script.contains("Assert-ExpectedListener"));
        assert!(script.contains("Clear-ProviderEnvironmentOverrides"));
        assert!(script.contains("Remove-Item -Path \"Env:$name\""));
        assert!(script.contains("VPR_DID_API_KEY"));
        assert!(script.contains("VPR_DID_AGENT_ID"));
        assert!(script.contains("VPR_OWNER_LAB_STT_API_KEY"));
        assert!(script.contains("VPR_OWNER_LAB_LLM_API_KEY"));
        assert!(script.contains("vpr-provider-credentials.exe"));
        assert!(script.contains("probe-did"));
        assert!(script.contains("ProtectedData]::Protect"));
        assert!(script.contains("ProtectedData]::Unprotect"));
        assert!(script.contains("DataProtectionScope]::CurrentUser"));
        assert!(script.contains("reviewed-persona.dpapi"));
        assert!(script.contains("C:\\VPR-RT0\\input\\reviewed-profile.json"));
        assert!(script.contains("C:\\VPR-RT0\\reviewed-profile.json"));
        assert!(script.contains("Get-CachedReviewedPersona"));
        assert!(script.contains("Convert-ToImportableReviewedPersona"));
        assert!(script.contains("/api/persona/reviewed/import"));
        assert!(script.contains("Reviewed Persona was found before restart but was not restored"));
        assert!(script.contains("MigrationSelfTest"));
        assert!(script.contains("owner_approved"));
        assert!(script.contains("bootstrap.egress_enabled"));
        assert!(script.contains("status.egress_enabled"));
    }
}
