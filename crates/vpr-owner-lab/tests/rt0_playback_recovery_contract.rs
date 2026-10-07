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
        "\"playback_recovery_triggered\"",
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
        APP.contains("\"provider_data_received\""),
        "LiveKit data receipt must be observable without storing provider payloads"
    );
    assert!(
        APP.contains("\"provider_playback_done_received\""),
        "normalized provider completion must be separately observable"
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


#[test]
fn strict_rt0_av_sync_never_promotes_html_media_clock_fallback() {
    for marker in [
        "if (rt0EvidenceMode)",
        "strict-rt0=rtp-playout-timestamp-only",
        "reference: null",
        "web_rtc_estimated_playout_timestamp",
    ] {
        assert!(
            APP.contains(marker),
            "strict RT0 A/V-sync fail-closed contract is missing marker: {marker}"
        );
    }
}


#[test]
fn prepared_livekit_interrupt_bypasses_http_roundtrip_on_the_media_stop_path() {
    let interrupt = APP
        .split("const interruptAvatar = async")
        .nth(1)
        .expect("interruptAvatar must exist");
    for marker in [
        "activeClientControl?.prepared_interrupt",
        "const fastProviderStop",
        "dispatchClientCommand(preparedInterrupt)",
        "const canonicalStop",
    ] {
        assert!(
            interrupt.contains(marker),
            "immediate provider STOP contract is missing marker: {marker}"
        );
    }
    let dispatch = interrupt
        .find("dispatchClientCommand(preparedInterrupt)")
        .expect("prepared STOP dispatch must exist");
    let canonical = interrupt
        .find("const canonicalStop")
        .expect("canonical cancellation must remain present");
    assert!(
        dispatch < canonical,
        "provider media STOP must be dispatched before waiting on canonical cancellation"
    );
}
