use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use vpr_evaluation::{
    ProviderStateManifest, sha256_hex, validate_candidate_sha, validate_live_provider_probe,
    validate_provider_state_manifest, validate_rt0_conversation_attempt_artifact,
};
use vpr_live_proof::{
    LiveConversationAttemptReceipt, LiveProviderProbeReceipt, prepare,
    run_live_conversation_attempt_with_avatar_probe, run_provider_probe_core,
    validate_live_conversation_inputs, validate_provider_probe_audio,
};

use super::{
    BoundaryError, CandidateRunReceipt, atomic_write, egress_authorized, emit_boundary,
    emit_conversation, emit_preflight, emit_probe, ensure_unique_paths, repo_snapshot,
    validated_input_path, validated_output_path, verify_snapshot, worktree_clean,
};

const CANDIDATE_BUNDLE_SCHEMA: &str = "rt0-live-proof-candidate-bundle-0.1";
const CANDIDATE_BUNDLE_EXTRACT_RECEIPT_SCHEMA: &str =
    "rt0-live-proof-candidate-bundle-extract-receipt-0.1";

#[derive(Serialize)]
struct CandidateBundle<'a> {
    schema_version: &'static str,
    candidate_sha: &'a str,
    provider_state_sha256: &'a str,
    provider_state: &'a ProviderStateManifest,
    provider_probe: &'a LiveProviderProbeReceipt,
    conversation_attempt: &'a LiveConversationAttemptReceipt,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateBundleInput {
    schema_version: String,
    candidate_sha: String,
    provider_state_sha256: String,
    provider_state: ProviderStateManifest,
    provider_probe: LiveProviderProbeReceipt,
    conversation_attempt: Value,
}

struct ExtractedCandidateArtifacts {
    candidate_sha: String,
    provider_state_sha256: String,
    provider_state: Vec<u8>,
    provider_probe: Vec<u8>,
    conversation_attempt: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateBundleExtractError {
    InvalidBundle,
    CandidateMismatch,
    ProviderStateInvalid,
    ProviderStateDigestMismatch,
    ProbeInvalid,
    ConversationInvalid,
    SerializationFailed,
}

impl CandidateBundleExtractError {
    const fn code(self) -> &'static str {
        match self {
            Self::InvalidBundle => "CANDIDATE_BUNDLE_INVALID",
            Self::CandidateMismatch => "CANDIDATE_BUNDLE_CANDIDATE_MISMATCH",
            Self::ProviderStateInvalid => "CANDIDATE_BUNDLE_PROVIDER_STATE_INVALID",
            Self::ProviderStateDigestMismatch => "CANDIDATE_BUNDLE_PROVIDER_STATE_DIGEST_MISMATCH",
            Self::ProbeInvalid => "CANDIDATE_BUNDLE_PROBE_INVALID",
            Self::ConversationInvalid => "CANDIDATE_BUNDLE_CONVERSATION_INVALID",
            Self::SerializationFailed => "CANDIDATE_BUNDLE_SERIALIZATION_FAILED",
        }
    }
}

#[derive(Serialize)]
struct CandidateBundleExtractReceipt<'a> {
    schema_version: &'static str,
    ok: bool,
    candidate_sha: &'a str,
    provider_state_sha256: &'a str,
    provider_state_artifact_sha256: String,
    provider_probe_artifact_sha256: String,
    conversation_attempt_artifact_sha256: String,
}

pub(super) fn run(
    probe_audio_path: &Path,
    profile_path: &Path,
    owner_audio_path: &Path,
    visitor_audio_path: &Path,
    bundle_path: &Path,
) -> Result<(), i32> {
    let snapshot = repo_snapshot()?;
    let probe_audio_path =
        validated_input_path(probe_audio_path, &snapshot.root).map_err(emit_boundary)?;
    let profile_path = validated_input_path(profile_path, &snapshot.root).map_err(emit_boundary)?;
    let owner_audio_path =
        validated_input_path(owner_audio_path, &snapshot.root).map_err(emit_boundary)?;
    let visitor_audio_path =
        validated_input_path(visitor_audio_path, &snapshot.root).map_err(emit_boundary)?;
    let bundle_path = validated_output_path(bundle_path, &snapshot.root).map_err(emit_boundary)?;

    let probe_audio =
        fs::read(&probe_audio_path).map_err(|_| emit_boundary(BoundaryError::InputReadFailed))?;
    let profile =
        fs::read(&profile_path).map_err(|_| emit_boundary(BoundaryError::InputReadFailed))?;
    let owner_audio =
        fs::read(&owner_audio_path).map_err(|_| emit_boundary(BoundaryError::InputReadFailed))?;
    let visitor_audio =
        fs::read(&visitor_audio_path).map_err(|_| emit_boundary(BoundaryError::InputReadFailed))?;

    validate_provider_probe_audio(&probe_audio).map_err(|error| emit_probe(&error))?;
    validate_live_conversation_inputs(&profile, &owner_audio, &visitor_audio)
        .map_err(emit_conversation)?;

    let prepared_probe = prepare(&snapshot.candidate, worktree_clean()?, egress_authorized())
        .map_err(emit_preflight)?;
    let provider_state_sha256 = prepared_probe.receipt().provider_state_sha256.clone();
    let provider_state = prepared_probe.receipt().provider_state.clone();
    let probe_core =
        run_provider_probe_core(prepared_probe, probe_audio).map_err(|error| emit_probe(&error))?;

    let prepared_conversation =
        prepare(&snapshot.candidate, worktree_clean()?, egress_authorized())
            .map_err(emit_preflight)?;
    if prepared_conversation.receipt().provider_state_sha256 != provider_state_sha256 {
        return Err(emit_boundary(BoundaryError::ProviderStateChanged));
    }
    let conversation_run = run_live_conversation_attempt_with_avatar_probe(
        prepared_conversation,
        &profile,
        owner_audio,
        visitor_audio,
    )
    .map_err(emit_conversation)?;
    let (receipt, avatar_probe) = conversation_run.into_parts();
    let probe = probe_core.with_avatar(avatar_probe);

    verify_snapshot(&snapshot).map_err(emit_preflight)?;
    let bundle = serde_json::to_vec_pretty(&CandidateBundle {
        schema_version: CANDIDATE_BUNDLE_SCHEMA,
        candidate_sha: &snapshot.candidate,
        provider_state_sha256: &provider_state_sha256,
        provider_state: &provider_state,
        provider_probe: &probe,
        conversation_attempt: &receipt,
    })
    .map_err(|_| 2)?;
    atomic_write(&bundle_path, &bundle).map_err(emit_boundary)?;
    if let Err(error) = verify_snapshot(&snapshot) {
        let _ = fs::remove_file(&bundle_path);
        return Err(emit_preflight(error));
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&CandidateRunReceipt {
            ok: true,
            candidate_sha: &snapshot.candidate,
            provider_state_sha256: &provider_state_sha256,
        })
        .map_err(|_| 2)?
    );
    Ok(())
}

pub(super) fn extract(
    bundle_path: &Path,
    provider_state_path: &Path,
    provider_probe_path: &Path,
    conversation_attempt_path: &Path,
) -> Result<(), i32> {
    let snapshot = repo_snapshot()?;
    let bundle_path = validated_input_path(bundle_path, &snapshot.root).map_err(emit_boundary)?;
    let provider_state_path =
        validated_output_path(provider_state_path, &snapshot.root).map_err(emit_boundary)?;
    let provider_probe_path =
        validated_output_path(provider_probe_path, &snapshot.root).map_err(emit_boundary)?;
    let conversation_attempt_path =
        validated_output_path(conversation_attempt_path, &snapshot.root).map_err(emit_boundary)?;
    ensure_unique_paths(&[
        bundle_path.as_path(),
        provider_state_path.as_path(),
        provider_probe_path.as_path(),
        conversation_attempt_path.as_path(),
    ])
    .map_err(emit_boundary)?;

    let bundle_bytes =
        fs::read(&bundle_path).map_err(|_| emit_boundary(BoundaryError::InputReadFailed))?;
    let extracted =
        extract_artifacts(&bundle_bytes, &snapshot.candidate).map_err(emit_extract_error)?;

    verify_snapshot(&snapshot).map_err(emit_preflight)?;
    let outputs = [
        (&provider_state_path, extracted.provider_state.as_slice()),
        (&provider_probe_path, extracted.provider_probe.as_slice()),
        (
            &conversation_attempt_path,
            extracted.conversation_attempt.as_slice(),
        ),
    ];
    let mut committed: Vec<&Path> = Vec::with_capacity(outputs.len());
    for (path, bytes) in outputs {
        if let Err(error) = atomic_write(path, bytes) {
            cleanup_outputs(&committed);
            return Err(emit_boundary(error));
        }
        committed.push(path);
    }
    if let Err(error) = verify_snapshot(&snapshot) {
        cleanup_outputs(&committed);
        return Err(emit_preflight(error));
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&CandidateBundleExtractReceipt {
            schema_version: CANDIDATE_BUNDLE_EXTRACT_RECEIPT_SCHEMA,
            ok: true,
            candidate_sha: &extracted.candidate_sha,
            provider_state_sha256: &extracted.provider_state_sha256,
            provider_state_artifact_sha256: sha256_hex(&extracted.provider_state),
            provider_probe_artifact_sha256: sha256_hex(&extracted.provider_probe),
            conversation_attempt_artifact_sha256: sha256_hex(&extracted.conversation_attempt),
        })
        .map_err(|_| 2)?
    );
    Ok(())
}

fn extract_artifacts(
    bundle_bytes: &[u8],
    exact_candidate_sha: &str,
) -> Result<ExtractedCandidateArtifacts, CandidateBundleExtractError> {
    validate_candidate_sha(exact_candidate_sha)
        .map_err(|_| CandidateBundleExtractError::CandidateMismatch)?;
    let bundle: CandidateBundleInput = serde_json::from_slice(bundle_bytes)
        .map_err(|_| CandidateBundleExtractError::InvalidBundle)?;
    if bundle.schema_version != CANDIDATE_BUNDLE_SCHEMA {
        return Err(CandidateBundleExtractError::InvalidBundle);
    }
    validate_candidate_sha(&bundle.candidate_sha)
        .map_err(|_| CandidateBundleExtractError::InvalidBundle)?;
    if bundle.candidate_sha != exact_candidate_sha {
        return Err(CandidateBundleExtractError::CandidateMismatch);
    }
    validate_provider_state_manifest(&bundle.provider_state)
        .map_err(|_| CandidateBundleExtractError::ProviderStateInvalid)?;

    let provider_state = serde_json::to_vec_pretty(&bundle.provider_state)
        .map_err(|_| CandidateBundleExtractError::SerializationFailed)?;
    let provider_state_sha256 = sha256_hex(&provider_state);
    if provider_state_sha256 != bundle.provider_state_sha256 {
        return Err(CandidateBundleExtractError::ProviderStateDigestMismatch);
    }

    validate_live_provider_probe(
        &bundle.provider_probe,
        exact_candidate_sha,
        &provider_state_sha256,
    )
    .map_err(|_| CandidateBundleExtractError::ProbeInvalid)?;
    let provider_probe = serde_json::to_vec_pretty(&bundle.provider_probe)
        .map_err(|_| CandidateBundleExtractError::SerializationFailed)?;

    let conversation_attempt = serde_json::to_vec_pretty(&bundle.conversation_attempt)
        .map_err(|_| CandidateBundleExtractError::SerializationFailed)?;
    validate_rt0_conversation_attempt_artifact(
        &conversation_attempt,
        exact_candidate_sha,
        &provider_state_sha256,
    )
    .map_err(|_| CandidateBundleExtractError::ConversationInvalid)?;

    Ok(ExtractedCandidateArtifacts {
        candidate_sha: bundle.candidate_sha,
        provider_state_sha256,
        provider_state,
        provider_probe,
        conversation_attempt,
    })
}

fn emit_extract_error(error: CandidateBundleExtractError) -> i32 {
    eprintln!(
        "{}",
        serde_json::json!({
            "ok": false,
            "code": error.code(),
        })
    );
    2
}

fn cleanup_outputs(paths: &[&Path]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vpr_evaluation::{
        AvatarProbeEvidence, LlmProbeEvidence, ProbeUsage, ProviderRole, ProviderStateBinding,
        RT0_PROVIDER_STATE_SCHEMA, SttProbeEvidence,
    };

    use super::*;

    fn provider_state() -> ProviderStateManifest {
        ProviderStateManifest {
            schema_version: RT0_PROVIDER_STATE_SCHEMA.into(),
            providers: vec![
                ProviderStateBinding {
                    role: ProviderRole::Stt,
                    provider: "deepgram".into(),
                    model_or_representation: "nova-3".into(),
                    configuration_fingerprint_sha256: "1".repeat(64),
                },
                ProviderStateBinding {
                    role: ProviderRole::Llm,
                    provider: "deepseek".into(),
                    model_or_representation: "deepseek-flash".into(),
                    configuration_fingerprint_sha256: "2".repeat(64),
                },
                ProviderStateBinding {
                    role: ProviderRole::Avatar,
                    provider: "d-id".into(),
                    model_or_representation: "expressive".into(),
                    configuration_fingerprint_sha256: "3".repeat(64),
                },
            ],
        }
    }

    fn bundle(candidate: &str) -> Vec<u8> {
        let provider_state = provider_state();
        let provider_state_bytes = serde_json::to_vec_pretty(&provider_state).unwrap();
        let provider_state_sha256 = sha256_hex(&provider_state_bytes);
        let usage = ProbeUsage {
            input_units: None,
            input_unit: None,
            output_units: None,
            output_unit: None,
            estimated_cost_microunits: None,
            provider_charge_microunits: None,
        };
        let probe = LiveProviderProbeReceipt {
            schema_version: vpr_evaluation::RT0_LIVE_PROVIDER_PROBE_SCHEMA.into(),
            candidate_sha: candidate.into(),
            provider_state_sha256: provider_state_sha256.clone(),
            input_audio_sha256: "4".repeat(64),
            input_audio_millis: 1_000,
            scope: "credentialed_provider_reachability_only".into(),
            conversation_evidence: false,
            output_delivery_proven: false,
            stt: SttProbeEvidence {
                latency_millis: 10,
                transcript_chars: 3,
                usage: usage.clone(),
            },
            llm: LlmProbeEvidence {
                latency_millis: 20,
                output_chars: 5,
                usage,
            },
            avatar: AvatarProbeEvidence {
                open_millis: 30,
                close_millis: 10,
            },
        };
        let turn = |audience: &str, marker: char| {
            json!({
                "audience": audience,
                "input_audio_sha256": marker.to_string().repeat(64),
                "transcript_sha256": "7".repeat(64),
                "transcript_chars": 3,
                "reply_sha256": "8".repeat(64),
                "reply_chars": 5,
                "locale": "ru-RU",
                "stt_millis": 10,
                "llm_millis": 20,
                "avatar_submit_millis": 30,
                "total_millis": 60,
                "estimated_cost_microunits": null,
                "provider_charge_microunits": null
            })
        };
        serde_json::to_vec_pretty(&json!({
            "schema_version": CANDIDATE_BUNDLE_SCHEMA,
            "candidate_sha": candidate,
            "provider_state_sha256": provider_state_sha256,
            "provider_state": provider_state,
            "provider_probe": probe,
            "conversation_attempt": {
                "schema_version": "rt0-live-conversation-attempt-0.1",
                "candidate_sha": candidate,
                "provider_state_sha256": provider_state_sha256,
                "profile_input_sha256": "9".repeat(64),
                "persona_id_sha256": "a".repeat(64),
                "persona_version": 1,
                "reviewed_claims": 1,
                "owner": turn("owner", '5'),
                "visitor": turn("visitor", '6'),
                "conversation_attempted": true,
                "provider_output_submitted": true,
                "browser_media_playback": "not_proven",
                "video_render": "not_proven",
                "human_review": "not_proven"
            }
        }))
        .unwrap()
    }

    #[test]
    fn extract_reconstructs_canonical_downstream_artifacts() {
        let candidate = "a".repeat(40);
        let extracted = extract_artifacts(&bundle(&candidate), &candidate).unwrap();

        assert_eq!(extracted.candidate_sha, candidate);
        assert_eq!(
            sha256_hex(&extracted.provider_state),
            extracted.provider_state_sha256
        );
        let probe: LiveProviderProbeReceipt =
            serde_json::from_slice(&extracted.provider_probe).unwrap();
        assert_eq!(probe.candidate_sha, candidate);
        validate_rt0_conversation_attempt_artifact(
            &extracted.conversation_attempt,
            &candidate,
            &extracted.provider_state_sha256,
        )
        .unwrap();
    }

    #[test]
    fn extract_rejects_candidate_mismatch() {
        let candidate = "a".repeat(40);
        assert_eq!(
            extract_artifacts(&bundle(&candidate), &"b".repeat(40))
                .err()
                .unwrap(),
            CandidateBundleExtractError::CandidateMismatch
        );
    }

    #[test]
    fn extract_rejects_provider_state_digest_tampering() {
        let candidate = "a".repeat(40);
        let bytes = bundle(&candidate);
        let mut value: Value = serde_json::from_slice(&bytes).unwrap();
        value["provider_state_sha256"] = Value::String("f".repeat(64));
        let tampered = serde_json::to_vec_pretty(&value).unwrap();

        assert_eq!(
            extract_artifacts(&tampered, &candidate).err().unwrap(),
            CandidateBundleExtractError::ProviderStateDigestMismatch
        );
    }
}
