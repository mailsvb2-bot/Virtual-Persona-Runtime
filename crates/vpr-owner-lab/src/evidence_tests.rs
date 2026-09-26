use super::*;
use crate::{LabTextResult, LabVoiceSegment, LabVoiceUsage};
use std::thread;
use std::time::Duration;

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
    assert!(snapshot.canonical_playback_proven);
    assert!(snapshot.av_sync_proven);
    assert_eq!(snapshot.av_sync_samples[0].absolute_offset_millis, 60);
    assert!(!json.contains("приватный транскрипт"));
    assert!(!json.contains("приватный ответ"));
    assert!(!json.contains("приватный текстовый ответ"));
    thread::sleep(Duration::from_millis(2));
    recorder.seal_session();
    let sealed_duration = recorder.snapshot().unwrap().session_duration_millis;
    assert!(sealed_duration > 0);
    thread::sleep(Duration::from_millis(2));
    assert_eq!(
        recorder.snapshot().unwrap().session_duration_millis,
        sealed_duration
    );
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
    assert!(recorder.session_started.is_some());
    assert!(recorder.sealed_duration_millis.is_none());
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
fn first_stream_segment_can_prove_playback_before_llm_completion() {
    let mut recorder = LabSessionEvidenceRecorder::default();
    recorder.begin_session(7, ParticipantRole::Owner).unwrap();
    recorder.begin_voice_request(1).unwrap();
    let segment = LabVoiceSegment {
        evidence_turn_sequence: 7,
        evidence_output_sequence: 1,
        client_command: None,
    };
    recorder.bind_voice_segment(1, &segment).unwrap();

    let audio_started = LabMediaEvidenceInput {
        session_sequence: 7,
        request_sequence: Some(1),
        kind: LabMediaEvidenceKind::AudioStarted,
        elapsed_millis: 120,
    };
    assert_eq!(
        recorder.prepare_canonical_playback(&audio_started),
        Ok((7, 1))
    );
    recorder
        .record_canonical_playback(&audio_started, 7, 1)
        .unwrap();
    let pending = recorder.snapshot().unwrap();
    assert_eq!(
        pending.voice_attempts[0].status,
        LabVoiceAttemptStatus::Pending
    );
    assert!(pending.voice_attempts[0].canonical_playback_confirmed);

    recorder.complete_voice_request(1, &voice_result()).unwrap();
    let completed = recorder.snapshot().unwrap();
    assert_eq!(
        completed.voice_attempts[0].status,
        LabVoiceAttemptStatus::Completed
    );
    assert!(completed.voice_attempts[0].canonical_playback_confirmed);

    let mut mismatched = LabSessionEvidenceRecorder::default();
    mismatched.begin_session(8, ParticipantRole::Owner).unwrap();
    mismatched.begin_voice_request(1).unwrap();
    mismatched.bind_voice_segment(1, &segment).unwrap();
    let mut wrong = voice_result();
    wrong.evidence_output_sequence = 2;
    assert_eq!(
        mismatched.complete_voice_request(1, &wrong),
        Err(LabEvidenceError::InvalidInput)
    );
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
