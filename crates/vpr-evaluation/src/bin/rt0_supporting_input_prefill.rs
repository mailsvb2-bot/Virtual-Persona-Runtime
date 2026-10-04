use std::{
    env, fs,
    fs::OpenOptions,
    io::Write,
    path::Path,
    process,
};

use serde_json::{Value, json};
use vpr_evaluation::{BoundLabSessionEvidenceAggregate, ProviderRole, bind_owner_lab_session_evidence};

const OUTPUT_SCHEMA: &str = "rt0-manual-supporting-observations-0.2";
const REVIEW_REQUIRED: &str = "REVIEW_REQUIRED";

fn main() {
    if let Err(error) = run(&env::args().skip(1).collect::<Vec<_>>()) {
        eprintln!("{error}");
        process::exit(2);
    }
}

fn run(args: &[String]) -> Result<(), String> {
    if args.len() < 5 {
        return Err(
            "usage: vpr-rt0-supporting-input-prefill <output.json> <provider-state.json> \
             <bound-session-aggregate.json> <exact-candidate-sha> <session-snapshot.json>..."
                .into(),
        );
    }

    let output = Path::new(&args[0]);

    let provider_state = read(&args[1], "provider state")?;
    let bound_bytes = read(&args[2], "bound session aggregate")?;
    let bound: BoundLabSessionEvidenceAggregate = serde_json::from_slice(&bound_bytes)
        .map_err(|_| "bound session aggregate JSON is invalid")?;
    let candidate = args[3].trim();

    let snapshots = args[4..]
        .iter()
        .map(|path| read(path, "session snapshot"))
        .collect::<Result<Vec<_>, _>>()?;
    let snapshot_refs = snapshots.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let recomputed = bind_owner_lab_session_evidence(&snapshot_refs, &provider_state, candidate)
        .map_err(|error| format!("runtime evidence recomputation failed: {error:?}"))?;
    if recomputed != bound {
        return Err("bound session aggregate does not match the supplied raw snapshots".into());
    }

    let value = prefill(&bound);
    let mut bytes =
        serde_json::to_vec_pretty(&value).map_err(|_| "prefill JSON serialization failed")?;
    bytes.push(b'\n');
    write_create_new(output, &bytes)?;

    println!(
        "RT0 reviewed-observations prefill created at {}. REVIEW_REQUIRED is intentionally \
         not a valid capture attestation; review every manual field before capture.",
        output.display()
    );
    Ok(())
}

fn prefill(bound: &BoundLabSessionEvidenceAggregate) -> Value {
    let estimated_roles = runtime_cost_roles(
        &bound.aggregate,
        bound.aggregate.estimated_cost_microunits,
    );
    let provider_charge_roles = runtime_cost_roles(
        &bound.aggregate,
        bound.aggregate.provider_charge_microunits,
    );

    json!({
        "schema_version": OUTPUT_SCHEMA,
        "attestation": REVIEW_REQUIRED,
        "acceptance": {
            "owner_happy_path": "failed",
            "visitor_happy_path": "failed",
            "correction_path": "failed",
            "failure_recovery_path": "failed",
            "revoke_deny_path": "failed"
        },
        "cost": {
            "estimated_cost_covered_provider_roles": estimated_roles,
            "provider_charge_covered_provider_roles": provider_charge_roles,
            "measured_duration_millis": bound.aggregate.session_duration_millis,
            "estimated_cost_microunits": bound.aggregate.estimated_cost_microunits,
            "provider_charge_microunits": bound.aggregate.provider_charge_microunits
        },
        "privacy_permissions": {
            "permission_suite": "failed",
            "accepted_private_context_leakage": 0,
            "accepted_false_owner_attribution": 0,
            "revocation": "failed",
            "egress_denial": "failed"
        },
        "human_evaluation": {
            "rubric_version": REVIEW_REQUIRED,
            "reviewer_count": 0,
            "owner_human_participant_verified": "failed",
            "visitor_distinct_non_owner_human_verified": "failed",
            "dimensions": {
                "voice_similarity": "missing",
                "voice_naturalness": "missing",
                "appearance_plausibility": "missing",
                "persona_similarity": "missing",
                "conversation_naturalness": "missing"
            },
            "usable_for_continuation": "failed"
        },
        "known_limitations": {
            "review_status": "failed",
            "body": "REVIEW_REQUIRED: document the known limitations observed for this exact candidate."
        }
    })
}

fn runtime_cost_roles(
    aggregate: &vpr_evaluation::LabSessionEvidenceAggregate,
    cost: Option<u64>,
) -> Vec<ProviderRole> {
    if cost.is_none() {
        return Vec::new();
    }
    if aggregate.completed_voice_attempts > 0 {
        return vec![ProviderRole::Stt, ProviderRole::Llm];
    }
    if aggregate.completed_text_attempts > 0 {
        return vec![ProviderRole::Llm];
    }
    Vec::new()
}

fn read(path: &str, label: &str) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|_| format!("{label} could not be read"))
}

fn write_create_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                "refusing to overwrite existing reviewed-observations input".to_string()
            } else {
                format!("prefill output could not be created: {error}")
            }
        })?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(format!("prefill output could not be created: {error}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpr_evaluation::{
        LabSessionEvidenceAggregate, RT0_OWNER_LAB_SESSION_AGGREGATE_SCHEMA,
        RT0_OWNER_LAB_SESSION_BINDING_SCHEMA, RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA,
    };

    fn bound(estimated: Option<u64>, charge: Option<u64>) -> BoundLabSessionEvidenceAggregate {
        BoundLabSessionEvidenceAggregate {
            schema_version: RT0_OWNER_LAB_SESSION_BINDING_SCHEMA.into(),
            candidate_sha: "a".repeat(40),
            provider_state_sha256: "b".repeat(64),
            snapshot_sha256: vec!["c".repeat(64)],
            aggregate: LabSessionEvidenceAggregate {
                schema_version: RT0_OWNER_LAB_SESSION_AGGREGATE_SCHEMA.into(),
                source_schema_version: RT0_OWNER_LAB_SESSION_EVIDENCE_SCHEMA.into(),
                sessions: 1,
                session_duration_millis: 42_000,
                completed_text_attempts: 0,
                failed_text_attempts: 0,
                completed_voice_attempts: 1,
                failed_voice_attempts: 0,
                canonical_playback_proven: true,
                av_sync_proven: false,
                text_first_meaningful_response: None,
                av_sync_absolute_offset: None,
                stt_latency: None,
                llm_latency: None,
                llm_first_meaningful_response: None,
                avatar_submit_latency: None,
                server_total_latency: None,
                first_meaningful_audio: None,
                interruption_stop: None,
                first_useful_video: None,
                recoverable_reconnect: None,
                estimated_cost_microunits: estimated,
                provider_charge_microunits: charge,
            },
        }
    }

    #[test]
    fn prefill_is_deliberately_non_promoting_and_uses_runtime_duration() {
        let value = prefill(&bound(Some(17), None));
        assert_eq!(value["attestation"], REVIEW_REQUIRED);
        assert_eq!(value["cost"]["measured_duration_millis"], 42_000);
        assert_eq!(
            value["cost"]["estimated_cost_covered_provider_roles"],
            json!(["stt", "llm"])
        );
        assert_eq!(
            value["cost"]["provider_charge_covered_provider_roles"],
            json!([])
        );
        assert_eq!(
            value["human_evaluation"]["visitor_distinct_non_owner_human_verified"],
            "failed"
        );
        assert_eq!(value["human_evaluation"]["reviewer_count"], 0);
    }

    #[test]
    fn unknown_runtime_cost_never_claims_provider_coverage() {
        let value = prefill(&bound(None, None));
        assert_eq!(
            value["cost"]["estimated_cost_covered_provider_roles"],
            json!([])
        );
        assert_eq!(
            value["cost"]["provider_charge_covered_provider_roles"],
            json!([])
        );
        assert!(value["cost"]["estimated_cost_microunits"].is_null());
        assert!(value["cost"]["provider_charge_microunits"].is_null());
    }

    #[test]
    fn text_only_runtime_cost_never_claims_stt_coverage() {
        let mut evidence = bound(Some(17), Some(19));
        evidence.aggregate.completed_voice_attempts = 0;
        evidence.aggregate.completed_text_attempts = 1;
        let value = prefill(&evidence);
        assert_eq!(
            value["cost"]["estimated_cost_covered_provider_roles"],
            json!(["llm"])
        );
        assert_eq!(
            value["cost"]["provider_charge_covered_provider_roles"],
            json!(["llm"])
        );
    }

    #[test]
    fn runtime_subtotals_never_claim_avatar_coverage() {
        let value = prefill(&bound(Some(17), Some(19)));
        for field in [
            "estimated_cost_covered_provider_roles",
            "provider_charge_covered_provider_roles",
        ] {
            let roles = value["cost"][field].as_array().unwrap();
            assert!(!roles.iter().any(|role| role == "avatar"));
        }
    }
}
