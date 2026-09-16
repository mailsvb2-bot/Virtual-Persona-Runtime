use std::{env, fs, path::Path};

use serde::Serialize;
use vpr_evaluation::{
    BoundGoldenReport, BoundLabSessionEvidenceAggregate, GoldenEvidenceBundle,
    LiveProviderProbeReceipt, ProviderStateManifest, Rt0ExitEvidence, Rt0ExitSupportingArtifacts,
    Rt0ExitVerificationContext,
    evaluate_verified_rt0_exit_evidence as evaluate_rt0_exit_evidence,
};

#[derive(Serialize)]
struct CliError<T: Serialize> {
    ok: bool,
    code: T,
}

struct SupportingArtifactBytes {
    ci: Vec<u8>,
    e2e: Vec<u8>,
    owner_conversation: Vec<u8>,
    visitor_conversation: Vec<u8>,
    acceptance: Vec<u8>,
    quality: Vec<u8>,
    cost: Vec<u8>,
    privacy_permissions: Vec<u8>,
    human_evaluation: Vec<u8>,
    known_limitations: Vec<u8>,
}

impl SupportingArtifactBytes {
    fn read(root: &Path) -> Result<Self, i32> {
        if !root.is_dir() {
            return input_invalid();
        }
        Ok(Self {
            ci: read_path(&root.join("ci-evidence.json"))?,
            e2e: read_path(&root.join("e2e-evidence.json"))?,
            owner_conversation: read_path(&root.join("owner-conversation.json"))?,
            visitor_conversation: read_path(&root.join("visitor-conversation.json"))?,
            acceptance: read_path(&root.join("acceptance.json"))?,
            quality: read_path(&root.join("quality.json"))?,
            cost: read_path(&root.join("cost.json"))?,
            privacy_permissions: read_path(&root.join("privacy-permissions.json"))?,
            human_evaluation: read_path(&root.join("human-evaluation.json"))?,
            known_limitations: read_path(&root.join("known-limitations.md"))?,
        })
    }

    fn as_verification(&self) -> Rt0ExitSupportingArtifacts<'_> {
        Rt0ExitSupportingArtifacts {
            ci: &self.ci,
            e2e: &self.e2e,
            owner_conversation: &self.owner_conversation,
            visitor_conversation: &self.visitor_conversation,
            acceptance: &self.acceptance,
            quality: &self.quality,
            cost: &self.cost,
            privacy_permissions: &self.privacy_permissions,
            human_evaluation: &self.human_evaluation,
            known_limitations: &self.known_limitations,
        }
    }
}

fn main() {
    if let Err(code) = run() {
        std::process::exit(code);
    }
}

fn run() -> Result<(), i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 11 {
        return usage();
    }

    let exit_evidence_path = &args[0];
    let golden_report_path = &args[1];
    let golden_evidence_path = &args[2];
    let provider_state_path = &args[3];
    let live_provider_probe_path = &args[4];
    let conversation_attempt_path = &args[5];
    let bound_session_aggregate_path = &args[6];
    let supporting_artifacts_dir = &args[7];
    let snapshot_paths = &args[8..args.len() - 2];
    let release_spec_path = &args[args.len() - 2];
    let candidate_sha = &args[args.len() - 1];

    let exit_evidence_bytes = read(exit_evidence_path)?;
    let golden_report_bytes = read(golden_report_path)?;
    let golden_evidence_bytes = read(golden_evidence_path)?;
    let provider_state_bytes = read(provider_state_path)?;
    let live_provider_probe_bytes = read(live_provider_probe_path)?;
    let conversation_attempt_bytes = read(conversation_attempt_path)?;
    let bound_session_aggregate_bytes = read(bound_session_aggregate_path)?;
    let supporting_artifacts = SupportingArtifactBytes::read(Path::new(supporting_artifacts_dir))?;
    let session_snapshot_bytes = snapshot_paths
        .iter()
        .map(|path| read(path))
        .collect::<Result<Vec<_>, _>>()?;
    let session_snapshot_artifacts: Vec<&[u8]> =
        session_snapshot_bytes.iter().map(Vec::as_slice).collect();
    let release_spec_bytes = read(release_spec_path)?;
    let evidence: Rt0ExitEvidence = parse(&exit_evidence_bytes)?;
    let golden_report: BoundGoldenReport = parse(&golden_report_bytes)?;
    let golden_evidence_bundle: GoldenEvidenceBundle = parse(&golden_evidence_bytes)?;
    let provider_state: ProviderStateManifest = parse(&provider_state_bytes)?;
    let live_provider_probe: LiveProviderProbeReceipt = parse(&live_provider_probe_bytes)?;
    let bound_session_aggregate: BoundLabSessionEvidenceAggregate =
        parse(&bound_session_aggregate_bytes)?;

    let report = match evaluate_rt0_exit_evidence(
        &evidence,
        &golden_report,
        Rt0ExitVerificationContext {
            exit_evidence_bytes: &exit_evidence_bytes,
            golden_report_bytes: &golden_report_bytes,
            golden_evidence_bundle: &golden_evidence_bundle,
            golden_evidence_bytes: &golden_evidence_bytes,
            provider_state: &provider_state,
            provider_state_bytes: &provider_state_bytes,
            live_provider_probe: &live_provider_probe,
            live_provider_probe_bytes: &live_provider_probe_bytes,
            conversation_attempt_bytes: &conversation_attempt_bytes,
            bound_session_aggregate: &bound_session_aggregate,
            bound_session_aggregate_bytes: &bound_session_aggregate_bytes,
            session_snapshot_artifacts: &session_snapshot_artifacts,
            release_spec_bytes: &release_spec_bytes,
            exact_candidate_sha: candidate_sha,
        },
        supporting_artifacts.as_verification(),
    ) {
        Ok(report) => report,
        Err(code) => {
            emit_error(code)?;
            return Err(2);
        }
    };

    println!("{}", serde_json::to_string_pretty(&report).map_err(|_| 2)?);
    if report.ready { Ok(()) } else { Err(1) }
}

fn read(path: &str) -> Result<Vec<u8>, i32> {
    read_path(Path::new(path))
}

fn read_path(path: &Path) -> Result<Vec<u8>, i32> {
    fs::read(path).map_err(|_| {
        let _ = emit_error("INPUT_INVALID");
        2
    })
}

fn input_invalid<T>() -> Result<T, i32> {
    let _ = emit_error("INPUT_INVALID");
    Err(2)
}

fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, i32> {
    serde_json::from_slice(bytes).map_err(|_| {
        let _ = emit_error("INPUT_INVALID");
        2
    })
}

fn emit_error<T: Serialize>(code: T) -> Result<(), i32> {
    eprintln!(
        "{}",
        serde_json::to_string(&CliError { ok: false, code }).map_err(|_| 2)?
    );
    Ok(())
}

fn usage() -> Result<(), i32> {
    eprintln!(
        "usage: vpr-rt0-exit-evidence <exit-evidence.json> <golden-report.json> <golden-evidence.json> <provider-state.json> <live-provider-probe.json> <conversation-attempt.json> <bound-session-aggregate.json> <supporting-evidence-dir> <session-snapshot.json>... <release-spec.md> <exact-candidate-sha>"
    );
    Err(2)
}
