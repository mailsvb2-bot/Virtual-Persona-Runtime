use super::*;
use crate::{LabTextResult, LabVoiceSegment, LabVoiceUsage};

fn text_result() -> LabTextResult {
    LabTextResult {
        reply: "приватный текстовый ответ".into(),
        locale: "ru-RU".into(),
        evidence_turn_sequence: 6,
        evidence_output_sequence: 2,
        first_meaningful_response_millis: 90,
        total_millis: 140,
        llm_usage: LabVoiceUsage {
            input_units: Some(10),
            output_units: Some(3),
            estimated_cost_microunits: Some(4),
            provider_charge_microunits: None,
        },
    }
}

fn voice_result() -> LabVoiceResult {
    LabVoiceResult {
        transcript: "приватный транскрипт".into(),
        reply: "приватный ответ".into(),
        locale: "ru".into(),
        evidence_turn_sequence: 7,
        evidence_output_sequence: 1,
        stt_millis: 100,
        llm_millis: 200,
        llm_first_meaningful_millis: 80,
        avatar_millis: 50,
        total_millis: 350,
        stt_usage: LabVoiceUsage {
            input_units: Some(1000),
            output_units: None,
            estimated_cost_microunits: Some(3),
            provider_charge_microunits: None,
        },
        llm_usage: LabVoiceUsage {
            input_units: Some(12),
            output_units: Some(4),
            estimated_cost_microunits: Some(5),
            provider_charge_microunits: Some(6),
        },
        client_command: None,
    }
}

fn assert_duration_freezes_after_seal(recorder: &mut LabSessionEvidenceRecorder) {
    recorder.seal_session();
    let sealed_duration = recorder.snapshot().unwrap().session_duration_millis;
    assert!(sealed_duration > 0);
    std::thread::sleep(std::time::Duration::from_millis(2));
    assert_eq!(
        recorder.snapshot().unwrap().session_duration_millis,
        sealed_duration
    );
}

fn assert_snapshot_is_bound_and_redacted(snapshot: &LabSessionEvidenceSnapshot) {
    let json = serde_json::to_string(snapshot).unwrap();
    assert_eq!(snapshot.text_attempts[0].canonical_turn_sequence, Some(6));
    assert_eq!(snapshot.text_attempts[0].canonical_output_sequence, Some(2));
    assert_eq!(
        snapshot.text_attempts[0].first_meaningful_response_millis,
        Some(90)
    );
    assert_eq!(snapshot.voice_attempts[0].canonical_turn_sequence, Some(7));
    assert_eq!(
        snapshot.voice_attempts[0].canonical_output_sequence,
        Some(1)
    );
    assert!(snapshot.voice_attempts[0].canonical_playback_confirmed);
    assert_eq!(
        snapshot.voice_attempts[0].llm_first_meaningful_millis,
        Some(80)
    );
    assert_eq!(snapshot.scope, RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE);
    assert_eq!(snapshot.participant_role, ParticipantRole::Owner);
    assert!(snapshot.session_duration_millis > 0);
    assert!(snapshot.canonical_playback_proven);
    assert!(snapshot.av_sync_proven);
    assert_eq!(snapshot.av_sync_samples[0].absolute_offset_millis, 60);
    assert!(!json.contains("приватный транскрипт"));
    assert!(!json.contains("приватный ответ"));
    assert!(!json.contains("приватный текстовый ответ"));
}

#[test]
fn session_reset_and_snapshot_are_payload_redacted() {
    let mut recorder = LabSessionEvidenceRecorder::default();
    recorder.begin_session(3, ParticipantRole::Owner).unwrap();
    recorder.begin_text_request(1).unwrap();
    recorder.complete_text_request(1, &text_result()).unwrap();
    recorder.begin_voice_request(1).unwrap();
    recorder.complete_voice_request(1, &voice_result()).unwrap();
    let audio_started = LabMediaEvidenceInput {
        session_sequence: 3,
        request_sequence: Some(1),
        kind: LabMediaEvidenceKind::AudioStarted,
        elapsed_millis: 410,
    };
    recorder.record_media(&audio_started).unwrap();
    assert_eq!(
        recorder.prepare_canonical_playback(&audio_started),
        Err(LabEvidenceError::InvalidInput)
    );
    let playback_completed = LabMediaEvidenceInput {
        kind: LabMediaEvidenceKind::PlaybackCompleted,
        elapsed_millis: 520,
        ..audio_started.clone()
    };
    assert_eq!(
        recorder.prepare_canonical_playback(&playback_completed),
        Ok((7, 1))
    );
    assert_eq!(
        recorder.record_canonical_playback(&playback_completed, 7, 2),
        Err(LabEvidenceError::InvalidState)
    );
    recorder
        .record_canonical_playback(&playback_completed, 7, 1)
        .unwrap();
    let av_sync = LabAvSyncEvidenceInput {
        session_sequence: 3,
        request_sequence: 1,
        sample_sequence: 1,
        reference: LabAvSyncReference::WebRtcEstimatedPlayoutTimestamp,
        absolute_offset_millis: 60,
    };
    recorder.record_av_sync(&av_sync).unwrap();
    assert_eq!(
        recorder.record_av_sync(&av_sync),
        Err(LabEvidenceError::DuplicateEvidence)
    );
    assert!(!recorder.snapshot().unwrap().av_sync_proven);
    for sample_sequence in 2..=RT0_AV_SYNC_SAMPLES_PER_REQUEST {
        recorder
            .record_av_sync(&LabAvSyncEvidenceInput {
                sample_sequence,
                absolute_offset_millis: 60 + u64::from(sample_sequence),
                ..av_sync.clone()
            })
            .unwrap();
    }
    let snapshot = recorder.snapshot().unwrap();
    assert_snapshot_is_bound_and_redacted(&snapshot);
    assert_duration_freezes_after_seal(&mut recorder);
    assert_eq!(
        recorder.begin_text_request(2),
        Err(LabEvidenceError::InvalidState)
    );
    assert_eq!(
        recorder.begin_voice_request(2),
        Err(LabEvidenceError::InvalidState)
    );
    assert_eq!(
        recorder.record_media(&LabMediaEvidenceInput {
            session_sequence: 3,
            request_sequence: None,
            kind: LabMediaEvidenceKind::VideoReady,
            elapsed_millis: 1,
        }),
        Err(LabEvidenceError::InvalidState)
    );
    recorder.begin_session(4, ParticipantRole::Visitor).unwrap();
    let reset = recorder.snapshot().unwrap();
    assert!(reset.text_attempts.is_empty());
    assert!(reset.voice_attempts.is_empty());
    assert_eq!(reset.participant_role, ParticipantRole::Visitor);
}

#[test]
fn text_attempts_fail_closed_and_keep_payloads_out_of_evidence() {
    let mut recorder = LabSessionEvidenceRecorder::default();
    recorder.begin_session(5, ParticipantRole::Visitor).unwrap();
    recorder.begin_text_request(1).unwrap();
    assert_eq!(
        recorder.begin_text_request(1),
        Err(LabEvidenceError::DuplicateEvidence)
    );
    recorder.complete_text_request(1, &text_result()).unwrap();
    assert_eq!(
        recorder.fail_text_request(1, "PROVIDER_TIMEOUT"),
        Err(LabEvidenceError::DuplicateEvidence)
    );

    recorder.begin_text_request(2).unwrap();
    recorder.fail_text_request(2, "PROVIDER_TIMEOUT").unwrap();
    let snapshot = recorder.snapshot().unwrap();
    assert_eq!(snapshot.text_attempts.len(), 2);
    assert_eq!(
        snapshot.text_attempts[0].status,
        LabTextAttemptStatus::Completed
    );
    assert_eq!(
        snapshot.text_attempts[1].status,
        LabTextAttemptStatus::Failed
    );
    let encoded = serde_json::to_string(&snapshot).unwrap();
    assert!(!encoded.contains("приватный текстовый ответ"));
}

#[test]
fn stale_unknown_and_duplicate_media_evidence_fail_closed() {
    let mut recorder = LabSessionEvidenceRecorder::default();
    recorder.begin_session(9, ParticipantRole::Owner).unwrap();
    recorder.begin_voice_request(5).unwrap();
    let event = LabMediaEvidenceInput {
        session_sequence: 9,
        request_sequence: None,
        kind: LabMediaEvidenceKind::VideoReady,
        elapsed_millis: 250,
    };
    recorder.record_media(&event).unwrap();
    assert_eq!(
        recorder.record_media(&event),
        Err(LabEvidenceError::DuplicateEvidence)
    );
    assert_eq!(
        recorder.record_media(&LabMediaEvidenceInput {
            session_sequence: 8,
            request_sequence: None,
            kind: LabMediaEvidenceKind::VideoReady,
            elapsed_millis: 10,
        }),
        Err(LabEvidenceError::InvalidState)
    );
    assert_eq!(
        recorder.record_media(&LabMediaEvidenceInput {
            session_sequence: 9,
            request_sequence: Some(999),
            kind: LabMediaEvidenceKind::InterruptionStopped,
            elapsed_millis: 10,
        }),
        Err(LabEvidenceError::InvalidState)
    );
}

#[test]
fn delivery_stage_media_is_request_scoped_and_duplicate_safe() {
    let mut recorder = LabSessionEvidenceRecorder::default();
    recorder.begin_session(19, ParticipantRole::Owner).unwrap();
    recorder.begin_voice_request(1).unwrap();
    recorder.complete_voice_request(1, &voice_result()).unwrap();

    for (kind, elapsed_millis) in [
        (LabMediaEvidenceKind::BackendCompleteReceived, 360),
        (LabMediaEvidenceKind::ClientDeliverySent, 410),
    ] {
        let event = LabMediaEvidenceInput {
            session_sequence: 19,
            request_sequence: Some(1),
            kind,
            elapsed_millis,
        };
        recorder.record_media(&event).unwrap();
        assert_eq!(
            recorder.record_media(&event),
            Err(LabEvidenceError::DuplicateEvidence)
        );
        assert_eq!(
            recorder.record_media(&LabMediaEvidenceInput {
                request_sequence: None,
                ..event.clone()
            }),
            Err(LabEvidenceError::InvalidInput)
        );
    }

    let snapshot = recorder.snapshot().unwrap();
    assert!(snapshot.media_events.iter().any(|event| {
        event.kind == LabMediaEvidenceKind::BackendCompleteReceived
            && event.request_sequence == Some(1)
            && event.elapsed_millis == 360
    }));
    assert!(snapshot.media_events.iter().any(|event| {
        event.kind == LabMediaEvidenceKind::ClientDeliverySent
            && event.request_sequence == Some(1)
            && event.elapsed_millis == 410
    }));
}

#[test]
fn session_attempt_and_media_retention_limits_fail_closed_at_exact_boundary() {
    let mut recorder = LabSessionEvidenceRecorder::default();
    recorder.begin_session(20, ParticipantRole::Owner).unwrap();

    for request_sequence in 1..=u64::try_from(RT0_OWNER_LAB_MAX_SESSION_ATTEMPTS).unwrap() {
        recorder.begin_text_request(request_sequence).unwrap();
    }
    assert_eq!(
        recorder.begin_voice_request(10_000),
        Err(LabEvidenceError::CapacityExceeded)
    );
    assert_eq!(
        recorder.snapshot().unwrap().text_attempts.len(),
        RT0_OWNER_LAB_MAX_SESSION_ATTEMPTS
    );

    recorder.seal_session();
    assert!(recorder.snapshot().is_ok());

    recorder
        .begin_session(21, ParticipantRole::Visitor)
        .unwrap();
    let reconnect = LabMediaEvidenceInput {
        session_sequence: 21,
        request_sequence: None,
        kind: LabMediaEvidenceKind::ReconnectRestored,
        elapsed_millis: 1,
    };
    for _ in 0..RT0_OWNER_LAB_MAX_MEDIA_EVENTS {
        recorder.record_media(&reconnect).unwrap();
    }
    assert_eq!(
        recorder.record_media(&reconnect),
        Err(LabEvidenceError::CapacityExceeded)
    );
    assert_eq!(
        recorder.snapshot().unwrap().media_events.len(),
        RT0_OWNER_LAB_MAX_MEDIA_EVENTS
    );
    assert_eq!(
        LabEvidenceError::CapacityExceeded.code(),
        "EVIDENCE_SESSION_CAPACITY_EXCEEDED"
    );
}

