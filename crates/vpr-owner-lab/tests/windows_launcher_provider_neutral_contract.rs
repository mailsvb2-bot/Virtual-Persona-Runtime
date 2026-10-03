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
fn windows_launcher_clears_avatar_environment_before_probe_and_process_launch() {
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

    let cleanup = WINDOWS_LAUNCHER
        .find("\n    Clear-ProviderEnvironmentOverrides\n")
        .expect("Windows launcher must invoke provider-environment cleanup");
    let probe = WINDOWS_LAUNCHER
        .find("& $credentialsExe probe-avatar")
        .expect("Windows launcher must invoke provider-neutral avatar preflight");
    let process_launch = WINDOWS_LAUNCHER
        .find("Start-Process -FilePath 'cmd.exe'")
        .expect("Windows launcher must start Owner Lab through the guarded launch path");

    assert!(
        cleanup < probe,
        "provider environment cleanup must run before avatar preflight"
    );
    assert!(
        probe < process_launch,
        "avatar preflight must complete before Owner Lab process launch"
    );
}
