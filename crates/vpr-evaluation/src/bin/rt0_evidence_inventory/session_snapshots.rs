use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

use serde::Serialize;
use vpr_evaluation::{
    BoundLabSessionEvidenceAggregate, LabSessionEvidenceSnapshot, Rt0ExitEvidence,
    bind_owner_lab_session_evidence, sha256_hex, validate_rt0_conversation_evidence_binding,
};

#[derive(Serialize)]
pub(super) struct SessionSnapshotItem {
    name: String,
    sha256: String,
    bound: bool,
}

#[derive(Serialize)]
pub(super) struct SessionSnapshotChecks {
    expected: usize,
    discovered: usize,
    all_expected_present: bool,
    no_unbound_snapshots: bool,
    binding_recomputed: bool,
    conversation_claims_bound: bool,
    files: Vec<SessionSnapshotItem>,
}

impl SessionSnapshotChecks {
    pub(super) fn complete(&self) -> bool {
        self.expected > 0
            && self.all_expected_present
            && self.no_unbound_snapshots
            && self.binding_recomputed
            && self.conversation_claims_bound
    }
}

type DiscoveredSnapshot = (String, String, Vec<u8>);

pub(super) fn collect_session_snapshot_checks(
    root: &Path,
    bound: Option<&BoundLabSessionEvidenceAggregate>,
    provider_state_bytes: Option<&[u8]>,
    candidate_sha: &str,
) -> SessionSnapshotChecks {
    let expected = bound
        .map(|receipt| receipt.snapshot_sha256.clone())
        .unwrap_or_default();
    let discovered = discover_snapshots(root);
    let (ordered, all_expected_present, no_unbound_snapshots) =
        order_bound_snapshots(&expected, &discovered);
    let binding_recomputed = recompute_binding(
        bound,
        provider_state_bytes,
        ordered.as_deref(),
        candidate_sha,
    );
    let conversation_claims_bound = validate_conversation_claims(
        root,
        provider_state_bytes,
        ordered.as_deref(),
        candidate_sha,
        binding_recomputed,
    );
    let expected_set: HashSet<&str> = expected.iter().map(String::as_str).collect();
    let files = discovered
        .into_iter()
        .map(|(name, sha256, _)| SessionSnapshotItem {
            bound: expected_set.contains(sha256.as_str()),
            name,
            sha256,
        })
        .collect::<Vec<_>>();
    SessionSnapshotChecks {
        expected: expected.len(),
        discovered: files.len(),
        all_expected_present,
        no_unbound_snapshots,
        binding_recomputed,
        conversation_claims_bound,
        files,
    }
}

fn discover_snapshots(root: &Path) -> Vec<DiscoveredSnapshot> {
    let mut discovered = Vec::new();
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
    discovered
}

fn order_bound_snapshots<'a>(
    expected: &[String],
    discovered: &'a [DiscoveredSnapshot],
) -> (Option<Vec<&'a [u8]>>, bool, bool) {
    let expected_set: HashSet<&str> = expected.iter().map(String::as_str).collect();
    let expected_unique = expected_set.len() == expected.len();
    let mut by_digest: HashMap<&str, &[u8]> = HashMap::new();
    let mut discovered_unique = true;
    for (_, digest, bytes) in discovered {
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
    let ordered = (all_expected_present && no_unbound_snapshots).then(|| {
        expected
            .iter()
            .filter_map(|digest| by_digest.get(digest.as_str()).copied())
            .collect()
    });
    (ordered, all_expected_present, no_unbound_snapshots)
}

fn recompute_binding(
    bound: Option<&BoundLabSessionEvidenceAggregate>,
    provider_state_bytes: Option<&[u8]>,
    ordered: Option<&[&[u8]]>,
    candidate_sha: &str,
) -> bool {
    match (bound, provider_state_bytes, ordered) {
        (Some(bound), Some(provider_state_bytes), Some(ordered)) => {
            bind_owner_lab_session_evidence(ordered, provider_state_bytes, candidate_sha)
                .is_ok_and(|recomputed| recomputed == *bound)
        }
        _ => false,
    }
}

fn validate_conversation_claims(
    root: &Path,
    provider_state_bytes: Option<&[u8]>,
    ordered: Option<&[&[u8]]>,
    candidate_sha: &str,
    binding_recomputed: bool,
) -> bool {
    if !binding_recomputed {
        return false;
    }
    let (Some(provider_state_bytes), Some(ordered)) = (provider_state_bytes, ordered) else {
        return false;
    };
    let Ok(conversation) = fs::read(root.join("conversation-attempt.json")) else {
        return false;
    };
    let Ok(exit_bytes) = fs::read(root.join("exit-evidence.json")) else {
        return false;
    };
    let Ok(exit) = serde_json::from_slice::<Rt0ExitEvidence>(&exit_bytes) else {
        return false;
    };
    validate_rt0_conversation_evidence_binding(
        &exit,
        &conversation,
        ordered,
        candidate_sha,
        &sha256_hex(provider_state_bytes),
    )
    .is_ok()
}
