use std::fs;
use std::path::Path;

use serde::Serialize;
use vpr_evaluation::ProviderStateManifest;
use vpr_live_proof::{
    LiveConversationAttemptReceipt, LiveProviderProbeReceipt, prepare, run_live_conversation_attempt,
    run_provider_probe, validate_live_conversation_inputs, validate_provider_probe_audio,
};

use super::{
    BoundaryError, CandidateRunReceipt, atomic_write, egress_authorized, emit_boundary,
    emit_conversation, emit_preflight, emit_probe, repo_snapshot, validated_input_path,
    validated_output_path, verify_snapshot, worktree_clean,
};

#[derive(Serialize)]
struct CandidateBundle<'a> {
    schema_version: &'static str,
    candidate_sha: &'a str,
    provider_state_sha256: &'a str,
    provider_state: &'a ProviderStateManifest,
    provider_probe: &'a LiveProviderProbeReceipt,
    conversation_attempt: &'a LiveConversationAttemptReceipt,
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

    let prepared_probe =
        prepare(&snapshot.candidate, worktree_clean()?, egress_authorized()).map_err(emit_preflight)?;
    let provider_state_sha256 = prepared_probe.receipt().provider_state_sha256.clone();
    let provider_state = prepared_probe.receipt().provider_state.clone();
    let probe =
        run_provider_probe(prepared_probe, probe_audio).map_err(|error| emit_probe(&error))?;

    let prepared_conversation =
        prepare(&snapshot.candidate, worktree_clean()?, egress_authorized())
            .map_err(emit_preflight)?;
    if prepared_conversation.receipt().provider_state_sha256 != provider_state_sha256 {
        return Err(emit_boundary(BoundaryError::ProviderStateChanged));
    }
    let receipt =
        run_live_conversation_attempt(prepared_conversation, &profile, owner_audio, visitor_audio)
            .map_err(emit_conversation)?;

    verify_snapshot(&snapshot).map_err(emit_preflight)?;
    let bundle = serde_json::to_vec_pretty(&CandidateBundle {
        schema_version: "rt0-live-proof-candidate-bundle-0.1",
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
