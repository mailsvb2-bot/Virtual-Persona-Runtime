use std::{env, fs, path::Path};

#[path = "rt0_evidence_inventory/session_snapshots.rs"]
mod session_snapshots;
#[path = "rt0_evidence_inventory/supporting.rs"]
mod supporting;

use serde::{Deserialize, Serialize};
use session_snapshots::{SessionSnapshotChecks, collect_session_snapshot_checks};
use supporting::SupportingArtifactBytes;
use vpr_evaluation::{
    BoundGoldenReport, BoundLabSessionEvidenceAggregate, LiveProviderProbeReceipt,
    ProviderStateManifest, RT0_EXIT_EVIDENCE_SCHEMA, RT0_LIVE_PROVIDER_PROBE_SCHEMA,
    RT0_OWNER_LAB_SESSION_BINDING_SCHEMA, RT0_PROVIDER_STATE_SCHEMA, Rt0ExitEvidence, sha256_hex,
    validate_rt0_exit_supporting_artifacts,
};

const SCHEMA: &str = "rt0-evidence-inventory-0.4";
const LIVE_CONVERSATION_ATTEMPT_SCHEMA: &str = "rt0-live-conversation-attempt-0.1";
const REQUIRED: &[&str] = &[
    "provider-state.json",
    "provider-probe.json",
    "conversation-attempt.json",
    "bound-session-aggregate.json",
    "bound-golden-report.json",
    "private-golden-evidence.json",
    "exit-evidence.json",
    "ci-evidence.json",
    "e2e-evidence.json",
    "owner-conversation.json",
    "visitor-conversation.json",
    "acceptance.json",
    "quality.json",
    "cost.json",
    "privacy-permissions.json",
    "human-evaluation.json",
    "known-limitations.md",
];

#[derive(Serialize)]
struct InventoryItem {
    name: &'static str,
    present: bool,
    syntax_valid: bool,
    sha256: Option<String>,
}
#[derive(Serialize)]
struct BindingChecks {
    candidate_sha_valid: bool,
    exit_candidate_matches: Option<bool>,
    exit_release_spec_matches_golden: Option<bool>,
    exit_golden_report_digest_matches: Option<bool>,
    exit_provider_state_matches: Option<bool>,
    exit_probe_digest_matches: Option<bool>,
    exit_conversation_digest_matches: Option<bool>,
    exit_session_digest_matches: Option<bool>,
    supporting_artifacts_bound: Option<bool>,
    golden_candidate_matches: Option<bool>,
    golden_provider_state_matches: Option<bool>,
    probe_candidate_matches: Option<bool>,
    probe_provider_state_matches: Option<bool>,
    conversation_candidate_matches: Option<bool>,
    conversation_provider_state_matches: Option<bool>,
    session_candidate_matches: Option<bool>,
    session_provider_state_matches: Option<bool>,
}

impl BindingChecks {
    fn complete(&self) -> bool {
        self.candidate_sha_valid
            && [
                self.exit_candidate_matches,
                self.exit_release_spec_matches_golden,
                self.exit_golden_report_digest_matches,
                self.exit_provider_state_matches,
                self.exit_probe_digest_matches,
                self.exit_conversation_digest_matches,
                self.exit_session_digest_matches,
                self.supporting_artifacts_bound,
                self.golden_candidate_matches,
                self.golden_provider_state_matches,
                self.probe_candidate_matches,
                self.probe_provider_state_matches,
                self.conversation_candidate_matches,
                self.conversation_provider_state_matches,
                self.session_candidate_matches,
                self.session_provider_state_matches,
            ]
            .into_iter()
            .all(|check| check == Some(true))
    }
}

struct ExitBindingChecks {
    candidate_matches: Option<bool>,
    release_spec_matches_golden: Option<bool>,
    golden_report_digest_matches: Option<bool>,
    provider_state_matches: Option<bool>,
    probe_digest_matches: Option<bool>,
    conversation_digest_matches: Option<bool>,
    session_digest_matches: Option<bool>,
    supporting_artifacts_bound: Option<bool>,
}

struct ExternalBindingChecks {
    golden_candidate: Option<bool>,
    golden_provider_state: Option<bool>,
    probe_candidate: Option<bool>,
    probe_provider_state: Option<bool>,
    conversation_candidate: Option<bool>,
    conversation_provider_state: Option<bool>,
    session_candidate: Option<bool>,
    session_provider_state: Option<bool>,
}

#[derive(Deserialize)]
struct ExternalBindingView {
    schema_version: String,
    candidate_sha: String,
    provider_state_sha256: String,
}

struct ParsedBindingArtifacts {
    provider: Option<ProviderStateManifest>,
    golden: Option<BoundGoldenReport>,
    probe: Option<LiveProviderProbeReceipt>,
    conversation: Option<ExternalBindingView>,
    session: Option<BoundLabSessionEvidenceAggregate>,
    exit: Option<Rt0ExitEvidence>,
    supporting: Option<SupportingArtifactBytes>,
    provider_state_bytes: Option<Vec<u8>>,
}

impl ParsedBindingArtifacts {
    fn read(root: &Path) -> Self {
        Self {
            provider: parse_optional(&root.join("provider-state.json")),
            golden: parse_optional(&root.join("bound-golden-report.json")),
            probe: parse_optional(&root.join("provider-probe.json")),
            conversation: parse_optional(&root.join("conversation-attempt.json")),
            session: parse_optional(&root.join("bound-session-aggregate.json")),
            exit: parse_optional(&root.join("exit-evidence.json")),
            supporting: SupportingArtifactBytes::read(root),
            provider_state_bytes: fs::read(root.join("provider-state.json")).ok(),
        }
    }

    fn schemas_valid(&self) -> bool {
        self.provider
            .as_ref()
            .is_some_and(|state| state.schema_version == RT0_PROVIDER_STATE_SCHEMA)
            && self
                .probe
                .as_ref()
                .is_some_and(|receipt| receipt.schema_version == RT0_LIVE_PROVIDER_PROBE_SCHEMA)
            && self
                .conversation
                .as_ref()
                .is_some_and(|receipt| receipt.schema_version == LIVE_CONVERSATION_ATTEMPT_SCHEMA)
            && self.session.as_ref().is_some_and(|receipt| {
                receipt.schema_version == RT0_OWNER_LAB_SESSION_BINDING_SCHEMA
            })
            && self
                .exit
                .as_ref()
                .is_some_and(|evidence| evidence.schema_version == RT0_EXIT_EVIDENCE_SCHEMA)
    }
}

struct InspectedBindings {
    checks: BindingChecks,
    valid: bool,
    session: Option<BoundLabSessionEvidenceAggregate>,
    provider_state_bytes: Option<Vec<u8>>,
}

#[derive(Serialize)]
struct InventoryReport {
    schema_version: &'static str,
    candidate_sha: String,
    evidence_dir: String,
    inventory_complete: bool,
    missing: Vec<&'static str>,
    items: Vec<InventoryItem>,
    bindings: BindingChecks,
    session_snapshots: SessionSnapshotChecks,
}

fn main() {
    if let Err(code) = run() {
        std::process::exit(code);
    }
}

fn run() -> Result<(), i32> {
    let mut args = env::args().skip(1);
    let Some(evidence_dir) = args.next() else {
        return usage();
    };
    let Some(candidate_sha) = args.next() else {
        return usage();
    };
    if args.next().is_some() {
        return usage();
    }
    let root = Path::new(&evidence_dir);
    let (items, missing, all_syntax_valid) = collect_inventory_items(root);
    let inspected = inspect_bindings(root, &candidate_sha);
    let session_snapshots = collect_session_snapshot_checks(
        root,
        inspected.session.as_ref(),
        inspected.provider_state_bytes.as_deref(),
        &candidate_sha,
    );
    let report = InventoryReport {
        schema_version: SCHEMA,
        candidate_sha,
        evidence_dir,
        inventory_complete: missing.is_empty()
            && all_syntax_valid
            && inspected.valid
            && session_snapshots.complete(),
        missing,
        items,
        bindings: inspected.checks,
        session_snapshots,
    };
    println!("{}", serde_json::to_string_pretty(&report).map_err(|_| 2)?);
    if report.inventory_complete {
        Ok(())
    } else {
        Err(1)
    }
}

fn inspect_bindings(root: &Path, candidate_sha: &str) -> InspectedBindings {
    let parsed = ParsedBindingArtifacts::read(root);
    let checks = inspect_binding_checks(root, candidate_sha, &parsed);
    let valid = parsed.schemas_valid() && checks.complete();
    InspectedBindings {
        checks,
        valid,
        session: parsed.session,
        provider_state_bytes: parsed.provider_state_bytes,
    }
}

fn inspect_binding_checks(
    root: &Path,
    candidate_sha: &str,
    parsed: &ParsedBindingArtifacts,
) -> BindingChecks {
    let provider_digest = parsed
        .provider_state_bytes
        .as_ref()
        .map(|bytes| sha256_hex(bytes));
    let exit = inspect_exit_binding_checks(root, candidate_sha, parsed, provider_digest.as_deref());
    let external =
        inspect_external_binding_checks(candidate_sha, parsed, provider_digest.as_deref());
    BindingChecks {
        candidate_sha_valid: valid_candidate_sha(candidate_sha),
        exit_candidate_matches: exit.candidate_matches,
        exit_release_spec_matches_golden: exit.release_spec_matches_golden,
        exit_golden_report_digest_matches: exit.golden_report_digest_matches,
        exit_provider_state_matches: exit.provider_state_matches,
        exit_probe_digest_matches: exit.probe_digest_matches,
        exit_conversation_digest_matches: exit.conversation_digest_matches,
        exit_session_digest_matches: exit.session_digest_matches,
        supporting_artifacts_bound: exit.supporting_artifacts_bound,
        golden_candidate_matches: external.golden_candidate,
        golden_provider_state_matches: external.golden_provider_state,
        probe_candidate_matches: external.probe_candidate,
        probe_provider_state_matches: external.probe_provider_state,
        conversation_candidate_matches: external.conversation_candidate,
        conversation_provider_state_matches: external.conversation_provider_state,
        session_candidate_matches: external.session_candidate,
        session_provider_state_matches: external.session_provider_state,
    }
}

fn inspect_exit_binding_checks(
    root: &Path,
    candidate_sha: &str,
    parsed: &ParsedBindingArtifacts,
    provider_digest: Option<&str>,
) -> ExitBindingChecks {
    ExitBindingChecks {
        candidate_matches: parsed
            .exit
            .as_ref()
            .map(|evidence| evidence.candidate_sha == candidate_sha),
        release_spec_matches_golden: parsed.exit.as_ref().zip(parsed.golden.as_ref()).map(
            |(evidence, report)| evidence.release_spec_sha256 == report.binding.release_spec_sha256,
        ),
        golden_report_digest_matches: exit_digest_matches(
            parsed.exit.as_ref(),
            root,
            "bound-golden-report.json",
            |evidence| &evidence.golden_report_sha256,
        ),
        provider_state_matches: parsed
            .exit
            .as_ref()
            .zip(provider_digest)
            .map(|(evidence, digest)| evidence.provider_state_sha256 == digest),
        probe_digest_matches: exit_digest_matches(
            parsed.exit.as_ref(),
            root,
            "provider-probe.json",
            |evidence| &evidence.live_provider_probe_sha256,
        ),
        conversation_digest_matches: exit_digest_matches(
            parsed.exit.as_ref(),
            root,
            "conversation-attempt.json",
            |evidence| &evidence.conversation_attempt_sha256,
        ),
        session_digest_matches: exit_digest_matches(
            parsed.exit.as_ref(),
            root,
            "bound-session-aggregate.json",
            |evidence| &evidence.bound_session_aggregate_sha256,
        ),
        supporting_artifacts_bound: parsed.exit.as_ref().zip(parsed.supporting.as_ref()).map(
            |(evidence, artifacts)| {
                validate_rt0_exit_supporting_artifacts(evidence, artifacts.as_verification())
                    .is_ok()
            },
        ),
    }
}

fn inspect_external_binding_checks(
    candidate_sha: &str,
    parsed: &ParsedBindingArtifacts,
    provider_digest: Option<&str>,
) -> ExternalBindingChecks {
    ExternalBindingChecks {
        golden_candidate: parsed
            .golden
            .as_ref()
            .map(|report| report.binding.candidate_sha == candidate_sha),
        golden_provider_state: parsed
            .golden
            .as_ref()
            .zip(provider_digest)
            .map(|(report, digest)| report.binding.provider_state_sha256 == digest),
        probe_candidate: parsed
            .probe
            .as_ref()
            .map(|receipt| receipt.candidate_sha == candidate_sha),
        probe_provider_state: parsed
            .probe
            .as_ref()
            .zip(provider_digest)
            .map(|(receipt, digest)| receipt.provider_state_sha256 == digest),
        conversation_candidate: parsed
            .conversation
            .as_ref()
            .map(|receipt| receipt.candidate_sha == candidate_sha),
        conversation_provider_state: parsed
            .conversation
            .as_ref()
            .zip(provider_digest)
            .map(|(receipt, digest)| receipt.provider_state_sha256 == digest),
        session_candidate: parsed
            .session
            .as_ref()
            .map(|receipt| receipt.candidate_sha == candidate_sha),
        session_provider_state: parsed
            .session
            .as_ref()
            .zip(provider_digest)
            .map(|(receipt, digest)| receipt.provider_state_sha256 == digest),
    }
}

fn exit_digest_matches(
    exit: Option<&Rt0ExitEvidence>,
    root: &Path,
    name: &str,
    declared: impl FnOnce(&Rt0ExitEvidence) -> &String,
) -> Option<bool> {
    exit.zip(file_digest(root, name))
        .map(|(evidence, digest)| declared(evidence) == &digest)
}

fn file_digest(root: &Path, name: &str) -> Option<String> {
    fs::read(root.join(name))
        .ok()
        .map(|bytes| sha256_hex(&bytes))
}

fn collect_inventory_items(root: &Path) -> (Vec<InventoryItem>, Vec<&'static str>, bool) {
    let mut items = Vec::with_capacity(REQUIRED.len());
    let mut missing = Vec::new();
    let mut all_syntax_valid = true;
    for &name in REQUIRED {
        let path = root.join(name);
        if let Ok(bytes) = fs::read(&path) {
            let syntax_valid = artifact_syntax_valid(name, &bytes);
            all_syntax_valid &= syntax_valid;
            items.push(InventoryItem {
                name,
                present: true,
                syntax_valid,
                sha256: Some(sha256_hex(&bytes)),
            });
        } else {
            missing.push(name);
            items.push(InventoryItem {
                name,
                present: false,
                syntax_valid: false,
                sha256: None,
            });
        }
    }
    (items, missing, all_syntax_valid)
}

fn artifact_syntax_valid(name: &str, bytes: &[u8]) -> bool {
    if Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        serde_json::from_slice::<serde_json::Value>(bytes).is_ok()
    } else {
        !bytes.iter().all(u8::is_ascii_whitespace)
    }
}

fn parse_optional<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn valid_candidate_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn usage() -> Result<(), i32> {
    eprintln!("usage: vpr-rt0-evidence-inventory <evidence-dir> <exact-candidate-sha>");
    Err(2)
}
