use super::*;

use crate::{LabVoiceResult, LabVoiceSegment, LabVoiceUsage};

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
fn audio_start_does_not_prove_full_playback_before_provider_completion() {
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
    recorder.record_media(&audio_started).unwrap();
    assert_eq!(
        recorder.prepare_canonical_playback(&audio_started),
        Err(LabEvidenceError::InvalidInput)
    );
    assert!(!recorder.snapshot().unwrap().voice_attempts[0].canonical_playback_confirmed);

    let playback_completed = LabMediaEvidenceInput {
        kind: LabMediaEvidenceKind::PlaybackCompleted,
        elapsed_millis: 600,
        ..audio_started
    };
    assert_eq!(
        recorder.prepare_canonical_playback(&playback_completed),
        Ok((7, 1))
    );
    recorder
        .record_canonical_playback(&playback_completed, 7, 1)
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
    let first_audio = LabMediaEvidenceInput {
        session_sequence: 8,
        request_sequence: Some(1),
        kind: LabMediaEvidenceKind::AudioStarted,
        elapsed_millis: 100,
    };
    recorder.record_media(&first_audio).unwrap();
    let first_done = LabMediaEvidenceInput {
        kind: LabMediaEvidenceKind::PlaybackCompleted,
        elapsed_millis: 500,
        ..first_audio
    };
    recorder
        .record_canonical_playback(&first_done, 11, 1)
        .unwrap();
    assert!(!recorder.snapshot().unwrap().canonical_playback_proven);

    let second_audio = LabMediaEvidenceInput {
        session_sequence: 8,
        request_sequence: Some(2),
        kind: LabMediaEvidenceKind::AudioStarted,
        elapsed_millis: 120,
    };
    recorder.record_media(&second_audio).unwrap();
    let second_done = LabMediaEvidenceInput {
        kind: LabMediaEvidenceKind::PlaybackCompleted,
        elapsed_millis: 520,
        ..second_audio
    };
    recorder
        .record_canonical_playback(&second_done, 12, 2)
        .unwrap();
    assert!(recorder.snapshot().unwrap().canonical_playback_proven);
    assert_eq!(
        recorder.record_canonical_playback(&second_done, 12, 2),
        Err(LabEvidenceError::DuplicateEvidence)
    );
}

#[test]
fn intentional_interruption_is_accounted_without_faking_full_playback() {
    let mut recorder = LabSessionEvidenceRecorder::default();
    recorder.begin_session(11, ParticipantRole::Owner).unwrap();

    for request in [1_u64, 2] {
        recorder.begin_voice_request(request).unwrap();
        let mut result = voice_result();
        result.evidence_turn_sequence = request + 20;
        result.evidence_output_sequence = request;
        recorder.complete_voice_request(request, &result).unwrap();
        recorder
            .record_media(&LabMediaEvidenceInput {
                session_sequence: 11,
                request_sequence: Some(request),
                kind: LabMediaEvidenceKind::AudioStarted,
                elapsed_millis: 100 + request,
            })
            .unwrap();
    }

    recorder
        .record_canonical_playback(
            &LabMediaEvidenceInput {
                session_sequence: 11,
                request_sequence: Some(1),
                kind: LabMediaEvidenceKind::PlaybackCompleted,
                elapsed_millis: 500,
            },
            21,
            1,
        )
        .unwrap();
    recorder
        .record_media(&LabMediaEvidenceInput {
            session_sequence: 11,
            request_sequence: Some(2),
            kind: LabMediaEvidenceKind::InterruptionStopped,
            elapsed_millis: 90,
        })
        .unwrap();

    let snapshot = recorder.snapshot().unwrap();
    assert!(snapshot.canonical_playback_proven);
    assert!(snapshot.voice_attempts[0].canonical_playback_confirmed);
    assert!(!snapshot.voice_attempts[1].canonical_playback_confirmed);

    let mut only_interrupted = LabSessionEvidenceRecorder::default();
    only_interrupted
        .begin_session(12, ParticipantRole::Owner)
        .unwrap();
    only_interrupted.begin_voice_request(1).unwrap();
    only_interrupted
        .complete_voice_request(1, &voice_result())
        .unwrap();
    only_interrupted
        .record_media(&LabMediaEvidenceInput {
            session_sequence: 12,
            request_sequence: Some(1),
            kind: LabMediaEvidenceKind::AudioStarted,
            elapsed_millis: 100,
        })
        .unwrap();
    only_interrupted
        .record_media(&LabMediaEvidenceInput {
            session_sequence: 12,
            request_sequence: Some(1),
            kind: LabMediaEvidenceKind::InterruptionStopped,
            elapsed_millis: 80,
        })
        .unwrap();
    assert!(
        !only_interrupted
            .snapshot()
            .unwrap()
            .canonical_playback_proven
    );
}

#[test]
fn av_sync_requires_completed_request_and_real_audio_start() {
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
    recorder.record_media(&audio_started).unwrap();
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
    assert!(!recorder.snapshot().unwrap().av_sync_proven);
    recorder
        .record_canonical_playback(
            &LabMediaEvidenceInput {
                kind: LabMediaEvidenceKind::PlaybackCompleted,
                elapsed_millis: 500,
                ..audio_started
            },
            7,
            1,
        )
        .unwrap();
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
fn av_sync_diagnostic_is_request_scoped_and_cannot_coexist_with_complete_proof() {
    let mut recorder = LabSessionEvidenceRecorder::default();
    recorder.begin_session(18, ParticipantRole::Owner).unwrap();
    recorder.begin_voice_request(1).unwrap();
    recorder.complete_voice_request(1, &voice_result()).unwrap();
    let audio_started = LabMediaEvidenceInput {
        session_sequence: 18,
        request_sequence: Some(1),
        kind: LabMediaEvidenceKind::AudioStarted,
        elapsed_millis: 400,
    };
    recorder.record_media(&audio_started).unwrap();
    recorder
        .record_canonical_playback(
            &LabMediaEvidenceInput {
                kind: LabMediaEvidenceKind::PlaybackCompleted,
                elapsed_millis: 650,
                ..audio_started.clone()
            },
            7,
            1,
        )
        .unwrap();

    let diagnostic = LabAvSyncDiagnosticInput {
        session_sequence: 18,
        request_sequence: 1,
        attempts: 50,
        audio_issue: Some(LabAvSyncTrackIssue::TimestampUnavailable),
        video_issue: None,
    };
    recorder.record_av_sync_diagnostic(&diagnostic).unwrap();
    assert_eq!(
        recorder.record_av_sync_diagnostic(&diagnostic),
        Err(LabEvidenceError::DuplicateEvidence)
    );
    assert_eq!(
        recorder.record_av_sync(&LabAvSyncEvidenceInput {
            session_sequence: 18,
            request_sequence: 1,
            sample_sequence: 1,
            reference: LabAvSyncReference::WebRtcEstimatedPlayoutTimestamp,
            absolute_offset_millis: 50,
        }),
        Err(LabEvidenceError::InvalidState)
    );
    let snapshot = recorder.snapshot().unwrap();
    assert!(!snapshot.av_sync_proven);
    assert_eq!(snapshot.av_sync_diagnostics.len(), 1);
    assert_eq!(
        snapshot.av_sync_diagnostics[0].audio_issue,
        Some(LabAvSyncTrackIssue::TimestampUnavailable)
    );

    let mut proven = LabSessionEvidenceRecorder::default();
    proven.begin_session(19, ParticipantRole::Owner).unwrap();
    proven.begin_voice_request(1).unwrap();
    proven.complete_voice_request(1, &voice_result()).unwrap();
    let proven_audio = LabMediaEvidenceInput {
        session_sequence: 19,
        ..audio_started
    };
    proven.record_media(&proven_audio).unwrap();
    proven
        .record_canonical_playback(
            &LabMediaEvidenceInput {
                kind: LabMediaEvidenceKind::PlaybackCompleted,
                elapsed_millis: 700,
                ..proven_audio
            },
            7,
            1,
        )
        .unwrap();
    for sample_sequence in 1..=RT0_AV_SYNC_SAMPLES_PER_REQUEST {
        proven
            .record_av_sync(&LabAvSyncEvidenceInput {
                session_sequence: 19,
                request_sequence: 1,
                sample_sequence,
                reference: LabAvSyncReference::WebRtcEstimatedPlayoutTimestamp,
                absolute_offset_millis: 50,
            })
            .unwrap();
    }
    assert_eq!(
        proven.record_av_sync_diagnostic(&LabAvSyncDiagnosticInput {
            session_sequence: 19,
            ..diagnostic
        }),
        Err(LabEvidenceError::InvalidState)
    );
}

#[test]
fn av_sync_retention_is_mathematically_bounded_by_session_attempt_budget() {
    assert_eq!(
        RT0_OWNER_LAB_MAX_AV_SYNC_SAMPLES,
        RT0_OWNER_LAB_MAX_SESSION_ATTEMPTS * RT0_AV_SYNC_SAMPLES_PER_REQUEST as usize
    );

    let mut recorder = LabSessionEvidenceRecorder::default();
    recorder.begin_session(22, ParticipantRole::Owner).unwrap();

    for request_sequence in 1..=u64::try_from(RT0_OWNER_LAB_MAX_SESSION_ATTEMPTS).unwrap() {
        recorder.begin_voice_request(request_sequence).unwrap();
        let mut result = voice_result();
        result.evidence_turn_sequence = request_sequence;
        result.evidence_output_sequence = request_sequence;
        recorder
            .complete_voice_request(request_sequence, &result)
            .unwrap();
        let audio_started = LabMediaEvidenceInput {
            session_sequence: 22,
            request_sequence: Some(request_sequence),
            kind: LabMediaEvidenceKind::AudioStarted,
            elapsed_millis: 1,
        };
        recorder.record_media(&audio_started).unwrap();
        recorder
            .record_canonical_playback(
                &LabMediaEvidenceInput {
                    kind: LabMediaEvidenceKind::PlaybackCompleted,
                    elapsed_millis: 2,
                    ..audio_started
                },
                request_sequence,
                request_sequence,
            )
            .unwrap();
        for sample_sequence in 1..=RT0_AV_SYNC_SAMPLES_PER_REQUEST {
            recorder
                .record_av_sync(&LabAvSyncEvidenceInput {
                    session_sequence: 22,
                    request_sequence,
                    sample_sequence,
                    reference: LabAvSyncReference::WebRtcEstimatedPlayoutTimestamp,
                    absolute_offset_millis: 1,
                })
                .unwrap();
        }
    }

    let snapshot = recorder.snapshot().unwrap();
    assert_eq!(
        snapshot.av_sync_samples.len(),
        RT0_OWNER_LAB_MAX_AV_SYNC_SAMPLES
    );
    assert_eq!(
        recorder.begin_voice_request(99_999),
        Err(LabEvidenceError::CapacityExceeded)
    );
}

#[test]
fn missing_rtp_diagnostic_survives_unconfirmed_playback_without_creating_false_proof() {
    let mut recorder = LabSessionEvidenceRecorder::default();
    recorder
        .bind_provenance(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();
    recorder.begin_session(26, ParticipantRole::Owner).unwrap();
    recorder.begin_voice_request(1).unwrap();
    recorder.complete_voice_request(1, &voice_result()).unwrap();

    let diagnostic = LabAvSyncDiagnosticInput {
        session_sequence: 26,
        request_sequence: 1,
        attempts: 50,
        audio_issue: Some(LabAvSyncTrackIssue::StatsUnavailable),
        video_issue: Some(LabAvSyncTrackIssue::TimestampUnavailable),
    };

    // A completed backend response alone does not establish actual audio.
    assert_eq!(
        recorder.record_av_sync_diagnostic(&diagnostic),
        Err(LabEvidenceError::InvalidState)
    );
    recorder
        .record_media(&LabMediaEvidenceInput {
            session_sequence: 26,
            request_sequence: Some(1),
            kind: LabMediaEvidenceKind::AudioStarted,
            elapsed_millis: 400,
        })
        .unwrap();
    recorder.record_av_sync_diagnostic(&diagnostic).unwrap();

    let snapshot = recorder.snapshot().unwrap();
    assert_eq!(snapshot.av_sync_diagnostics.len(), 1);
    assert_eq!(snapshot.av_sync_diagnostics[0].request_sequence, 1);
    assert_eq!(
        snapshot.av_sync_diagnostics[0].video_issue,
        Some(LabAvSyncTrackIssue::TimestampUnavailable)
    );
    assert!(!snapshot.voice_attempts[0].canonical_playback_confirmed);
    assert!(!snapshot.canonical_playback_proven);
    assert!(!snapshot.av_sync_proven);
    assert!(
        !serde_json::to_string(&snapshot)
            .unwrap()
            .contains("приватный")
    );

    // The diagnostic is scoped, unique and excludes later claims of completed proof.
    assert_eq!(
        recorder.record_av_sync_diagnostic(&diagnostic),
        Err(LabEvidenceError::DuplicateEvidence)
    );
    assert_eq!(
        recorder.record_av_sync(&LabAvSyncEvidenceInput {
            session_sequence: 26,
            request_sequence: 1,
            sample_sequence: 1,
            reference: LabAvSyncReference::WebRtcEstimatedPlayoutTimestamp,
            absolute_offset_millis: 75,
        }),
        Err(LabEvidenceError::InvalidState)
    );
    assert_eq!(
        recorder.record_av_sync_diagnostic(&LabAvSyncDiagnosticInput {
            session_sequence: 25,
            ..diagnostic.clone()
        }),
        Err(LabEvidenceError::InvalidState)
    );

    // A real release-evidence consumer must accept the typed failure without
    // counting the completed backend turn as heard or A/V-synchronized.
    std::thread::sleep(std::time::Duration::from_millis(2));
    recorder.seal_session();
    let aggregate = vpr_evaluation::aggregate_owner_lab_session_evidence(&[
        recorder.snapshot().unwrap(),
    ])
    .unwrap();
    assert_eq!(aggregate.completed_voice_attempts, 1);
    assert!(!aggregate.canonical_playback_proven);
    assert!(!aggregate.av_sync_proven);
    assert_eq!(aggregate.av_sync_absolute_offset, None);
}
