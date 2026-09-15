use std::{env, fs, path::Path};

use serde::{Deserialize, Serialize};
use vpr_evaluation::{
    BoundGoldenReport, BoundLabSessionEvidenceAggregate, LiveProviderProbeReceipt,
    ProviderStateManifest, RT0_LIVE_PROVIDER_PROBE_SCHEMA, RT0_OWNER_LAB_SESSION_BINDING_SCHEMA,
    RT0_PROVIDER_STATE_SCHEMA, sha256_hex,
};

const SCHEMA: &str = "rt0-evidence-inventory-0.1";
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
    let provider_digest = fs::read(root.join("provider-state.json"))
        .ok()
        .map(|bytes| sha256_hex(&bytes));
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
        inventory_complete: missing.is_empty() && all_syntax_valid && binding_ok,
        missing,
        items,
        bindings,
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
