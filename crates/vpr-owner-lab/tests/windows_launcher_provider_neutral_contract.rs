const WINDOWS_LAUNCHER: &str = include_str!("../../../scripts/windows-owner-lab-restart.ps1");

#[test]
fn windows_launcher_probes_the_selected_avatar_provider() {
    assert!(
        WINDOWS_LAUNCHER.contains("& $credentialsExe probe-avatar"),
        "Windows launcher must probe the selected avatar adapter"
    );
    assert!(
        !WINDOWS_LAUNCHER.contains("& $credentialsExe probe-did"),
        "Windows launcher must not hard-code D-ID as the launch preflight"
    );
}

#[test]
fn windows_launcher_clears_both_supported_avatar_environment_bindings() {
    for variable in [
        "VPR_OWNER_LAB_AVATAR_PROVIDER",
        "VPR_DID_API_KEY",
        "VPR_DID_AGENT_ID",
        "VPR_LOCAL_AVATAR_ENDPOINT",
        "VPR_LOCAL_AVATAR_API_TOKEN",
    ] {
        assert!(
            WINDOWS_LAUNCHER.contains(variable),
            "Windows launcher must clear inherited {variable} before secure-profile preflight"
        );
    }
}
