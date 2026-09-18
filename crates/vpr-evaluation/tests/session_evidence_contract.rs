use vpr_evaluation::{
    LabAvSyncEvidence, LabAvSyncReference, LabMediaEvidence, LabMediaEvidenceKind,
    LabSessionAggregateError, LabSessionEvidenceSnapshot, LabVoiceAttemptEvidence,
    LabVoiceAttemptStatus, ParticipantRole, RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE,
    RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA, SessionUsageEvidence,
    aggregate_owner_lab_session_evidence,
};

fn usage(cost: Option<u64>, charge: Option<u64>) -> SessionUsageEvidence {
    SessionUsageEvidence {
        input_units: Some(10),
        output_units: Some(4),
        estimated_cost_microunits: cost,
        provider_charge_microunits: charge,
    }
}

fn completed_text(request: u64, base: u64) -> LabTextAttemptEvidence {
    LabTextAttemptEvidence {
        request_sequence: request,
        canonical_turn_sequence: Some(request + 50),
        canonical_output_sequence: Some(request + 60),
        status: LabTextAttemptStatus::Completed,
        failure_code: None,
        first_meaningful_response_millis: Some(base + 50),
        server_total_millis: Some(base + 150),
        llm_usage: Some(usage(Some(2), Some(3))),
    }
}

fn completed(request: u64, base: u64) -> LabVoiceAttemptEvidence {
    LabVoiceAttemptEvidence {
        request_sequence: request,
        canonical_turn_sequence: Some(request + 100),
        canonical_output_sequence: Some(request + 200),
        canonical_playback_confirmed: true,
        status: LabVoiceAttemptStatus::Completed,
        failure_code: None,
        stt_millis: Some(base),
        llm_millis: Some(base + 100),
        avatar_millis: Some(base + 20),
        server_total_millis: Some(base + 250),
        stt_usage: Some(usage(Some(2), Some(3))),
        llm_usage: Some(usage(Some(5), Some(7))),
    }
}

fn snapshot(session: u64, request: u64, base: u64) -> LabSessionEvidenceSnapshot {
    LabSessionEvidenceSnapshot {
        schema_version: RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA.into(),
        scope: RT0_OWNER_LAB_MEDIA_EVIDENCE_SCOPE.into(),
        session_sequence: session,
        participant_role: ParticipantRole::Owner,
        canonical_playback_proven: true,
        av_sync_proven: true,
        text_attempts: vec![completed_text(request, base)],
        voice_attempts: vec![completed(request, base)],
        media_events: vec![
            LabMediaEvidence {
                request_sequence: Some(request),
                kind: LabMediaEvidenceKind::AudioStarted,
                elapsed_millis: base + 300,
            },
            LabMediaEvidence {
                request_sequence: Some(request),
                kind: LabMediaEvidenceKind::InterruptionStopped,
                elapsed_millis: base / 10,
            },
            LabMediaEvidence {
                request_sequence: None,
                kind: LabMediaEvidenceKind::VideoReady,
                elapsed_millis: base + 400,
            },
            LabMediaEvidence {
                request_sequence: None,
                kind: LabMediaEvidenceKind::ReconnectRestored,
                elapsed_millis: base + 500,
            },
        ],
        av_sync_samples: vec![
            LabAvSyncEvidence {
                request_sequence: request,
                sample_sequence: 1,
                reference: LabAvSyncReference::WebRtcEstimatedPlayoutTimestamp,
                absolute_offset_millis: base / 10 + 10,
            },
            LabAvSyncEvidence {
                request_sequence: request,
                sample_sequence: 2,
                reference: LabAvSyncReference::WebRtcEstimatedPlayoutTimestamp,
                absolute_offset_millis: base / 10 + 20,
            },
            LabAvSyncEvidence {
                request_sequence: request,
                sample_sequence: 3,
                reference: LabAvSyncReference::WebRtcEstimatedPlayoutTimestamp,
                absolute_offset_millis: base / 10 + 30,
            },
        ],
    }
}

#[test]
fn aggregate_computes_deterministic_distributions_and_complete_cost_only() {
    let aggregate =
        aggregate_owner_lab_session_evidence(&[snapshot(1, 1, 100), snapshot(2, 1, 300)]).unwrap();
    assert_eq!(aggregate.sessions, 2);
    assert_eq!(aggregate.completed_text_attempts, 2);
    assert_eq!(aggregate.failed_text_attempts, 0);
    assert_eq!(aggregate.completed_voice_attempts, 2);
    assert_eq!(
        aggregate.text_first_meaningful_response.unwrap().p50,
        150
    );
    assert_eq!(
        aggregate.text_first_meaningful_response.unwrap().p95,
        350
    );
    assert_eq!(aggregate.first_meaningful_audio.unwrap().p50, 400);
    assert_eq!(aggregate.first_meaningful_audio.unwrap().p95, 600);
    assert_eq!(aggregate.interruption_stop.unwrap().p95, 30);
    assert_eq!(aggregate.first_useful_video.unwrap().p95, 700);
    assert_eq!(aggregate.recoverable_reconnect.unwrap().p95, 800);
    assert_eq!(aggregate.estimated_cost_microunits, Some(18));
    assert_eq!(aggregate.provider_charge_microunits, Some(26));
    assert!(aggregate.canonical_playback_proven);
    assert!(aggregate.av_sync_proven);
    assert_eq!(aggregate.av_sync_absolute_offset.unwrap().p50, 40);
    assert_eq!(aggregate.av_sync_absolute_offset.unwrap().p95, 60);
}

#[test]
fn partial_cost_never_becomes_a_fake_complete_total() {
    let mut input = snapshot(1, 1, 100);
    input.voice_attempts[0]
        .llm_usage
        .as_mut()
        .unwrap()
        .estimated_cost_microunits = None;
    let aggregate = aggregate_owner_lab_session_evidence(&[input]).unwrap();
    assert_eq!(aggregate.estimated_cost_microunits, None);
    assert_eq!(aggregate.provider_charge_microunits, Some(10));
}

#[test]
fn text_attempts_are_exact_and_fail_closed() {
    let mut pending = snapshot(50, 1, 100);
    pending.text_attempts[0].status = LabTextAttemptStatus::Pending;
    assert_eq!(
        aggregate_owner_lab_session_evidence(&[pending]),
        Err(LabSessionAggregateError::IncompleteAttempt)
    );

    let mut impossible = snapshot(51, 1, 100);
    impossible.text_attempts[0].first_meaningful_response_millis = Some(300);
    impossible.text_attempts[0].server_total_millis = Some(200);
    assert_eq!(
        aggregate_owner_lab_session_evidence(&[impossible]),
        Err(LabSessionAggregateError::InvalidSnapshot)
    );

    let mut failed = snapshot(52, 1, 100);
    failed.text_attempts[0] = LabTextAttemptEvidence {
        request_sequence: 1,
        canonical_turn_sequence: None,
        canonical_output_sequence: None,
        status: LabTextAttemptStatus::Failed,
        failure_code: Some("PROVIDER_TIMEOUT".into()),
        first_meaningful_response_millis: None,
        server_total_millis: None,
        llm_usage: None,
    };
    let aggregate = aggregate_owner_lab_session_evidence(&[failed]).unwrap();
    assert_eq!(aggregate.completed_text_attempts, 0);
    assert_eq!(aggregate.failed_text_attempts, 1);
    assert_eq!(aggregate.text_first_meaningful_response, None);
}

#[test]
fn duplicate_pending_and_forged_snapshots_fail_closed() {
    let original = snapshot(1, 1, 100);
    assert_eq!(
        aggregate_owner_lab_session_evidence(&[original.clone(), original]),
        Err(LabSessionAggregateError::DuplicateSnapshot)
    );

    let mut pending = snapshot(1, 1, 100);
    pending.voice_attempts[0].status = LabVoiceAttemptStatus::Pending;
    assert_eq!(
        aggregate_owner_lab_session_evidence(&[pending]),
        Err(LabSessionAggregateError::IncompleteAttempt)
    );

    let mut forged = snapshot(1, 1, 100);
    forged.voice_attempts[0].canonical_playback_confirmed = false;
    assert_eq!(
        aggregate_owner_lab_session_evidence(&[forged]),
        Err(LabSessionAggregateError::InvalidSnapshot)
    );
}

#[test]
fn failed_attempts_must_be_clean_and_media_cannot_claim_failed_output() {
    let mut dirty = snapshot(1, 1, 100);
    let attempt = &mut dirty.voice_attempts[0];
    attempt.status = LabVoiceAttemptStatus::Failed;
    attempt.failure_code = Some("TURN_CANCELLED".into());
    assert_eq!(
        aggregate_owner_lab_session_evidence(&[dirty]),
        Err(LabSessionAggregateError::InvalidSnapshot)
    );

    let mut failed = snapshot(1, 1, 100);
    failed.voice_attempts[0] = LabVoiceAttemptEvidence {
        request_sequence: 1,
        canonical_turn_sequence: None,
        canonical_output_sequence: None,
        canonical_playback_confirmed: false,
        status: LabVoiceAttemptStatus::Failed,
        failure_code: Some("TURN_CANCELLED".into()),
        stt_millis: None,
        llm_millis: None,
        avatar_millis: None,
        server_total_millis: None,
        stt_usage: None,
        llm_usage: None,
    };
    assert_eq!(
        aggregate_owner_lab_session_evidence(&[failed]),
        Err(LabSessionAggregateError::InvalidMediaEvidence)
    );
}

#[test]
fn same_session_sequence_with_different_content_is_still_duplicate() {
    let first = snapshot(21, 1, 100);
    let mut second = snapshot(21, 2, 110);
    second.media_events.clear();
    assert_eq!(
        aggregate_owner_lab_session_evidence(&[first, second]),
        Err(LabSessionAggregateError::DuplicateSnapshot)
    );
}

#[test]
fn aggregate_requires_playback_proof_from_every_session() {
    let proven = snapshot(31, 1, 100);
    let mut unproven = snapshot(32, 1, 200);
    unproven.canonical_playback_proven = false;
    unproven.voice_attempts[0].canonical_playback_confirmed = false;
    unproven.av_sync_proven = false;
    unproven.av_sync_samples.clear();
    unproven.media_events.retain(|event| {
        !matches!(
            event.kind,
            LabMediaEvidenceKind::AudioStarted | LabMediaEvidenceKind::InterruptionStopped
        )
    });
    let aggregate = aggregate_owner_lab_session_evidence(&[proven, unproven]).unwrap();
    assert!(!aggregate.canonical_playback_proven);
}

#[test]
fn interruption_requires_observed_audio_for_the_same_request() {
    let mut snapshot = snapshot(22, 1, 100);
    snapshot
        .media_events
        .retain(|event| event.kind != LabMediaEvidenceKind::AudioStarted);
    assert_eq!(
        aggregate_owner_lab_session_evidence(&[snapshot]),
        Err(LabSessionAggregateError::InvalidMediaEvidence)
    );
}

#[test]
fn av_sync_requires_canonical_playback_and_unique_request_scoped_samples() {
    let mut missing = snapshot(41, 1, 100);
    missing.av_sync_proven = false;
    missing.av_sync_samples.clear();
    let aggregate = aggregate_owner_lab_session_evidence(&[missing]).unwrap();
    assert!(!aggregate.av_sync_proven);
    assert_eq!(aggregate.av_sync_absolute_offset, None);

    let mut partial = snapshot(42, 1, 100);
    partial.av_sync_proven = false;
    partial.av_sync_samples.pop();
    let aggregate = aggregate_owner_lab_session_evidence(&[partial]).unwrap();
    assert!(!aggregate.av_sync_proven);
    assert_eq!(aggregate.av_sync_absolute_offset.unwrap().samples, 2);

    let mut duplicate = snapshot(42, 1, 100);
    duplicate
        .av_sync_samples
        .push(duplicate.av_sync_samples[0].clone());
    assert_eq!(
        aggregate_owner_lab_session_evidence(&[duplicate]),
        Err(LabSessionAggregateError::InvalidMediaEvidence)
    );

    let mut cross_request = snapshot(43, 1, 100);
    cross_request.av_sync_samples[0].request_sequence = 99;
    assert_eq!(
        aggregate_owner_lab_session_evidence(&[cross_request]),
        Err(LabSessionAggregateError::InvalidMediaEvidence)
    );
}
