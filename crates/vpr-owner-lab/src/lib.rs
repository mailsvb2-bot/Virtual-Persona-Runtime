mod evidence;
mod owner_capture;
mod owner_context;
mod persona_persistence;
mod provider_credentials;
mod providers;
mod state;
#[cfg(windows)]
mod windows_secure_store;

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

pub use persona_persistence::{persist_reviewed_capture, restore_reviewed_persona};

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
        assert!(script.contains("VPR_OWNER_LAB_AVATAR_PROVIDER"));
        assert!(script.contains("VPR_LOCAL_AVATAR_ENDPOINT"));
        assert!(script.contains("VPR_LOCAL_AVATAR_API_TOKEN"));
        assert!(script.contains("VPR_OWNER_LAB_STT_API_KEY"));
        assert!(script.contains("VPR_OWNER_LAB_LLM_API_KEY"));
        assert!(script.contains("vpr-provider-credentials.exe"));
        assert!(script.contains("probe-avatar"));
        assert!(script.contains("bootstrap.egress_enabled"));
        assert!(script.contains("status.egress_enabled"));
        assert!(script.contains("Assert-SafeToRestart"));
        assert!(!script.contains("Export-ReviewedPersonaIfPresent"));
        assert!(!script.contains("Restore-ReviewedPersona"));
        assert!(!script.contains("/api/persona/create"));
        assert!(!script.contains("/api/persona/reviewed"));
    }
}
