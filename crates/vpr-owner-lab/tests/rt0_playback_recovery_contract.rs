const APP: &str = include_str!("../ui/src/app.ts");

#[test]
fn strict_rt0_recovers_from_missing_provider_playback_done_without_promoting_evidence() {
    for marker in [
        "UNCONFIRMED_PLAYBACK_SILENCE_RECOVERY_MILLIS = 3_000",
        "voice.responseComplete",
        "voice.audioStarted",
        "!voice.providerPlaybackDone",
        "!voice.interrupted",
        "voice.playbackSilenceStartedAt !== null",
        "voice.playbackRecoveryTriggered = true",
        "interruptAvatar(false)",
        "playback_completed не засчитан",
    ] {
        assert!(
            APP.contains(marker),
            "RT0 missing-playback recovery contract is missing marker: {marker}"
        );
    }

    let recovery = APP
        .find("voice.playbackRecoveryTriggered = true")
        .expect("recovery trigger must exist");
    let recovery_tail = &APP[recovery..];
    let interrupt = recovery_tail
        .find("interruptAvatar(false)")
        .expect("recovery must fail closed through provider interrupt");
    let next_completion_post = recovery_tail.find("\"playback_completed\"");

    assert!(
        next_completion_post.is_none() || interrupt < next_completion_post.unwrap(),
        "recovery must not synthesize playback_completed before the fail-closed interrupt"
    );
}

#[test]
fn provider_playback_done_remains_the_only_success_path_for_completion_evidence() {
    assert!(
        APP.contains("normalized?.kind === \"playback_done\""),
        "provider completion event handler must remain present"
    );
    assert!(
        APP.contains("voice.providerPlaybackDone = true"),
        "provider completion must set the canonical provider acknowledgement"
    );
    assert!(
        APP.contains("ensurePlaybackCompletionEvidence(voice)"),
        "provider acknowledgement must remain the playback-completion evidence path"
    );
}
