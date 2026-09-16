use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::Path,
};

use serde::{Deserialize, Serialize};
use vpr_evaluation::{
    BoundGoldenReport, BoundLabSessionEvidenceAggregate, LabSessionEvidenceSnapshot,
    LiveProviderProbeReceipt, ProviderStateManifest, RT0_LIVE_PROVIDER_PROBE_SCHEMA,
    RT0_OWNER_LAB_SESSION_BINDING_SCHEMA, RT0_PROVIDER_STATE_SCHEMA,
    bind_owner_lab_session_evidence, sha256_hex,
};

const SCHEMA: &str = "rt0-evidence-inventory-0.2";
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
struct SessionSnapshotItem {
    name: String,
    sha256: String,
    bound: bool,
}

#[derive(Serialize)]
struct SessionSnapshotChecks {
    expected: usize,
    discovered: usize,
    all_expected_present: bool,
    no_unbound_snapshots: bool,
    binding_recomputed: bool,
    files: Vec<SessionSnapshotItem>,
}

impl SessionSnapshotChecks {
    fn complete(&self) -> bool {
        self.expected > 0
            && self.all_expected_present
            && self.no_unbound_snapshots
            && self.binding_recomputed
    }
}

#[derive(Serialize)]
struct BindingChecks {
    candidate_sha_valid: bool,
    golden_candidate_matches: Option<bool>,
    golden_provider_state_matches: Option<bool>,
    probe_candidate_matches: Option<bool>,
    probe_provider_state_matches: Option<bool>,
    conversation_candidate_matches: Option<bool>,
    conversation_provider_state_matches: Option<bool>,
    session_candidate_matches: Option<bool>,
    session_provider_state_matches: Option<bool>,
}

#[derive(Deserialize)]
struct ExternalBindingView {
    schema_version: String,
    candidate_sha: String,
    provider_state_sha256: String,
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

    let provider = parse_optional::<ProviderStateManifest>(&root.join("provider-state.json"));
    let golden = parse_optional::<BoundGoldenReport>(&root.join("bound-golden-report.json"));
    let probe = parse_optional::<LiveProviderProbeReceipt>(&root.join("provider-probe.json"));
    let conversation =
        parse_optional::<ExternalBindingView>(&root.join("conversation-attempt.json"));
    let session = parse_optional::<BoundLabSessionEvidenceAggregate>(
        &root.join("bound-session-aggregate.json"),
    );
    let provider_state_bytes = fs::read(root.join("provider-state.json")).ok();
    let provider_digest = provider_state_bytes.as_ref().map(|bytes| sha256_hex(bytes));
    let bindings = BindingChecks {
        candidate_sha_valid: valid_candidate_sha(&candidate_sha),
        golden_candidate_matches: golden
            .as_ref()
            .map(|report| report.binding.candidate_sha == candidate_sha),
        golden_provider_state_matches: golden
            .as_ref()
            .zip(provider_digest.as_ref())
            .map(|(report, digest)| report.binding.provider_state_sha256 == digest.as_str()),
        probe_candidate_matches: probe
            .as_ref()
            .map(|receipt| receipt.candidate_sha == candidate_sha),
        probe_provider_state_matches: probe
            .as_ref()
            .zip(provider_digest.as_ref())
            .map(|(receipt, digest)| receipt.provider_state_sha256 == digest.as_str()),
        conversation_candidate_matches: conversation
            .as_ref()
            .map(|receipt| receipt.candidate_sha == candidate_sha),
        conversation_provider_state_matches: conversation
            .as_ref()
            .zip(provider_digest.as_ref())
            .map(|(receipt, digest)| receipt.provider_state_sha256 == digest.as_str()),
        session_candidate_matches: session
            .as_ref()
            .map(|receipt| receipt.candidate_sha == candidate_sha),
        session_provider_state_matches: session
            .as_ref()
            .zip(provider_digest.as_ref())
            .map(|(receipt, digest)| receipt.provider_state_sha256 == digest.as_str()),
    };

    let session_snapshots = collect_session_snapshot_checks(
        root,
        session.as_ref(),
        provider_state_bytes.as_deref(),
        &candidate_sha,
    );

    let binding_ok = provider
        .as_ref()
        .is_some_and(|state| state.schema_version == RT0_PROVIDER_STATE_SCHEMA)
        && probe
            .as_ref()
            .is_some_and(|receipt| receipt.schema_version == RT0_LIVE_PROVIDER_PROBE_SCHEMA)
        && conversation
            .as_ref()
            .is_some_and(|receipt| receipt.schema_version == LIVE_CONVERSATION_ATTEMPT_SCHEMA)
        && session
            .as_ref()
            .is_some_and(|receipt| receipt.schema_version == RT0_OWNER_LAB_SESSION_BINDING_SCHEMA)
        && bindings.candidate_sha_valid
        && bindings.golden_candidate_matches.unwrap_or(false)
        && bindings.golden_provider_state_matches.unwrap_or(false)
        && bindings.probe_candidate_matches.unwrap_or(false)
        && bindings.probe_provider_state_matches.unwrap_or(false)
        && bindings.conversation_candidate_matches.unwrap_or(false)
        && bindings
            .conversation_provider_state_matches
            .unwrap_or(false)
        && bindings.session_candidate_matches.unwrap_or(false)
        && bindings.session_provider_state_matches.unwrap_or(false);
    let report = InventoryReport {
        schema_version: SCHEMA,
        candidate_sha,
        evidence_dir,
        inventory_complete: missing.is_empty()
            && all_syntax_valid
            && binding_ok
            && session_snapshots.complete(),
        missing,
        items,
        bindings,
        session_snapshots,
    };
    println!("{}", serde_json::to_string_pretty(&report).map_err(|_| 2)?);
    if report.inventory_complete {
        Ok(())
    } else {
        Err(1)
    }
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

fn collect_session_snapshot_checks(
    root: &Path,
    bound: Option<&BoundLabSessionEvidenceAggregate>,
    provider_state_bytes: Option<&[u8]>,
    candidate_sha: &str,
) -> SessionSnapshotChecks {
    let expected: Vec<String> = bound
        .map(|receipt| receipt.snapshot_sha256.clone())
        .unwrap_or_default();
    let expected_set: HashSet<&str> = expected.iter().map(String::as_str).collect();
    let expected_unique = expected_set.len() == expected.len();

    let mut discovered = Vec::<(String, String, Vec<u8>)>::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file()
                || !path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            if serde_json::from_slice::<LabSessionEvidenceSnapshot>(&bytes).is_err() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            discovered.push((name.to_owned(), sha256_hex(&bytes), bytes));
        }
    }
    discovered.sort_by(|left, right| left.0.cmp(&right.0));

    let mut by_digest: HashMap<&str, &[u8]> = HashMap::new();
    let mut discovered_unique = true;
    for (_, digest, bytes) in &discovered {
        if by_digest
            .insert(digest.as_str(), bytes.as_slice())
            .is_some()
        {
            discovered_unique = false;
        }
    }
    let all_expected_present = expected_unique
        && expected
            .iter()
            .all(|digest| by_digest.contains_key(digest.as_str()));
    let no_unbound_snapshots = discovered_unique
        && discovered.len() == expected.len()
        && discovered
            .iter()
            .all(|(_, digest, _)| expected_set.contains(digest.as_str()));

    let binding_recomputed = if all_expected_present && no_unbound_snapshots {
        match (bound, provider_state_bytes) {
            (Some(bound), Some(provider_state_bytes)) => {
                let ordered: Vec<&[u8]> = expected
                    .iter()
                    .filter_map(|digest| by_digest.get(digest.as_str()).copied())
                    .collect();
                bind_owner_lab_session_evidence(&ordered, provider_state_bytes, candidate_sha)
                    .is_ok_and(|recomputed| recomputed == *bound)
            }
            _ => false,
        }
    } else {
        false
    };

    let files = discovered
        .into_iter()
        .map(|(name, sha256, _)| SessionSnapshotItem {
            bound: expected_set.contains(sha256.as_str()),
            name,
            sha256,
        })
        .collect();

    SessionSnapshotChecks {
        expected: expected.len(),
        discovered: files.len(),
        all_expected_present,
        no_unbound_snapshots,
        binding_recomputed,
        files,
    }
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
