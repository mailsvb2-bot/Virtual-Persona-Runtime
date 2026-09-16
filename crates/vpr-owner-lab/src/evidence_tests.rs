use super::*;
use crate::LabVoiceUsage;

fn voice_result() -> LabVoiceResult {
    LabVoiceResult {
        transcript: "приватный транскрипт".into(),
        reply: "приватный ответ".into(),
        locale: "ru".into(),
        evidence_turn_sequence: 7,
        evidence_output_sequence: 1,
        stt_millis: 100,
        llm_millis: 200,
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
    }
}

#[test]
fn session_reset_and_snapshot_are_payload_redacted() {
    let mut recorder = LabSessionEvidenceRecorder::default();
    recorder.begin_session(3, ParticipantRole::Owner).unwrap();
    recorder.begin_voice_request(1).unwrap();
    recorder.complete_voice_request(1, &voice_result()).unwrap();
    let audio_started = LabMediaEvidenceInput {
        session_sequence: 3,
        request_sequence: Some(1),
        kind: LabMediaEvidenceKind::AudioStarted,
        elapsed_millis: 410,
    };
    assert_eq!(
        recorder.record_media(&audio_started),
        Err(LabEvidenceError::InvalidState)
    );
    assert_eq!(
        recorder.prepare_canonical_playback(&audio_started),
        Ok((7, 1))
    );
    assert_eq!(
        recorder.record_canonical_playback(&audio_started, 7, 2),
        Err(LabEvidenceError::InvalidState)
    );
    recorder
        .record_canonical_playback(&audio_started, 7, 1)
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
    let json = serde_json::to_string(&snapshot).unwrap();
    assert_eq!(snapshot.voice_attempts[0].canonical_turn_sequence, Some(7));
    assert_eq!(
        snapshot.voice_attempts[0].canonical_output_sequence,
        Some(1)
    );
    assert!(snapshot.voice_attempts[0].canonical_playback_confirmed);
    assert_eq!(snapshot.scope, RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE);
    assert_eq!(snapshot.participant_role, ParticipantRole::Owner);
    assert!(snapshot.canonical_playback_proven);
    assert!(snapshot.av_sync_proven);
    assert_eq!(snapshot.av_sync_samples[0].absolute_offset_millis, 60);
    assert!(!json.contains("приватный транскрипт"));
    assert!(!json.contains("приватный ответ"));
    recorder.seal_session();
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
    assert!(reset.voice_attempts.is_empty());
    assert_eq!(reset.participant_role, ParticipantRole::Visitor);
}

#[test]
fn every_completed_voice_request_requires_its_own_runtime_playback_confirmation() {
    let mut recorder = LabSessionEvidenceRecorder::default();
    recorder.begin_session(8, ParticipantRole::Owner).unwrap();
    for request in [1, 2] {
        recorder.begin_voice_request(request).unwrap();
        let mut result = voice_result();
        result.evidence_turn_sequence = request + 10;
        result.evidence_output_sequence = request;
        recorder.complete_voice_request(request, &result).unwrap();
    }
    let first = LabMediaEvidenceInput {
        session_sequence: 8,
        request_sequence: Some(1),
        kind: LabMediaEvidenceKind::AudioStarted,
        elapsed_millis: 100,
    };
    recorder.record_canonical_playback(&first, 11, 1).unwrap();
    assert!(!recorder.snapshot().unwrap().canonical_playback_proven);

    let second = LabMediaEvidenceInput {
        session_sequence: 8,
        request_sequence: Some(2),
        kind: LabMediaEvidenceKind::AudioStarted,
        elapsed_millis: 120,
    };
    recorder.record_canonical_playback(&second, 12, 2).unwrap();
    assert!(recorder.snapshot().unwrap().canonical_playback_proven);
    assert_eq!(
        recorder.record_canonical_playback(&second, 12, 2),
        Err(LabEvidenceError::DuplicateEvidence)
    );
}

#[test]
fn av_sync_requires_completed_canonical_playback_for_the_same_request() {
    let mut recorder = LabSessionEvidenceRecorder::default();
    recorder.begin_session(12, ParticipantRole::Owner).unwrap();
    recorder.begin_voice_request(1).unwrap();
    let sample = LabAvSyncEvidenceInput {
        session_sequence: 12,
        request_sequence: 1,
        sample_sequence: 1,
        reference: LabAvSyncReference::WebRtcEstimatedPlayoutTimestamp,
        absolute_offset_millis: 40,
    };
    assert_eq!(
        recorder.record_av_sync(&sample),
        Err(LabEvidenceError::InvalidState)
    );
    recorder.complete_voice_request(1, &voice_result()).unwrap();
    assert_eq!(
        recorder.record_av_sync(&sample),
        Err(LabEvidenceError::InvalidState)
    );
    let audio_started = LabMediaEvidenceInput {
        session_sequence: 12,
        request_sequence: Some(1),
        kind: LabMediaEvidenceKind::AudioStarted,
        elapsed_millis: 100,
    };
    recorder
        .record_canonical_playback(&audio_started, 7, 1)
        .unwrap();
    recorder.record_av_sync(&sample).unwrap();
    assert!(!recorder.snapshot().unwrap().av_sync_proven);
    for sample_sequence in 2..=RT0_AV_SYNC_SAMPLES_PER_REQUEST {
        recorder
            .record_av_sync(&LabAvSyncEvidenceInput {
                sample_sequence,
                ..sample.clone()
            })
            .unwrap();
    }
    assert!(recorder.snapshot().unwrap().av_sync_proven);
    assert_eq!(
        recorder.record_av_sync(&LabAvSyncEvidenceInput {
            sample_sequence: RT0_AV_SYNC_SAMPLES_PER_REQUEST + 1,
            ..sample
        }),
        Err(LabEvidenceError::InvalidInput)
    );
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
