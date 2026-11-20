//! Canonical owner account persistence and concurrency regressions.
//! All writes target the SAME Persona envelope and same sibling transaction.
use super::*;
use crate::owner_context::{DurableOwnerClaimRevisionSnapshot, DurableReviewedOwnerClaimSnapshot};
use std::time::{SystemTime, UNIX_EPOCH};

fn reviewed(persona_id: &str, version: u64) -> DurableReviewedOwnerContextSnapshot {
    DurableReviewedOwnerContextSnapshot {
        persona_id: persona_id.to_owned(),
        persona_version: version,
        claims: vec![DurableReviewedOwnerClaimSnapshot {
            claim_id: "communication-preference".into(),
            history_complete: true,
            revisions: (1..=version)
                .map(|revision| DurableOwnerClaimRevisionSnapshot {
                    revision,
                    statement: format!("Revision {revision}"),
                    kind: "preference".into(),
                    source: "owner".into(),
                    verification: if revision == 1 {
                        "unverified".into()
                    } else {
                        "owner_verified".into()
                    },
                    derivation: "direct".into(),
                })
                .collect(),
        }],
    }
}

fn three_owners() -> (PersistedPersonaV2, String, String, String) {
    let mut record = PersistedPersonaV2::new(reviewed("legacy-primary", 3));
    let (primary, primary_token) = OwnerWorkspaceRecord::provision().unwrap();
    record.workspace = Some(primary);
    let (_, b_token) = provision_in_record(&mut record, reviewed("persona-b", 3)).unwrap();
    let (_, c_token) = provision_in_record(&mut record, reviewed("persona-c", 3)).unwrap();
    record.clone().validate().unwrap();
    (record, primary_token, b_token, c_token)
}

#[test]
fn authenticating_owner_b_never_returns_owner_a_or_c_persona() {
    let (mut record, a, b, c) = three_owners();
    assert_eq!(status_for(&record, &a).unwrap().persona_id, "legacy-primary");
    assert_eq!(status_for(&record, &b).unwrap().persona_id, "persona-b");
    assert_eq!(status_for(&record, &c).unwrap().persona_id, "persona-c");
    assert!(status_for(&record, &"0".repeat(64)).is_err());
    assert!(provision_in_record(&mut record, reviewed("persona-b", 4)).is_err());
    assert!(correction_in_record(&mut record, &a, reviewed("persona-b", 4)).is_err());
    assert!(correction_in_record(&mut record, &b, reviewed("persona-c", 4)).is_err());
    assert_eq!(status_for(&record, &b).unwrap().persona_version, 3);
    let serialized = serde_json::to_string(&record).unwrap();
    let decoded: PersistedPersonaV2 = serde_json::from_str(&serialized).unwrap();
    decoded.clone().validate().unwrap();
    assert_eq!(status_for(&decoded, &c).unwrap().persona_id, "persona-c");
}

#[test]
fn legacy_primary_commit_preserves_all_additional_workspaces() {
    let (old, primary_token, b_token, c_token) = three_owners();
    let raw = serde_json::to_string(&old).unwrap();
    let mut corrected_primary = reviewed("legacy-primary", 4);
    corrected_primary.claims[0].revisions[3].statement = "Owner A correction".into();
    let next = PersistedPersonaV2::reconciled(corrected_primary, Some(&raw)).unwrap();
    assert_eq!(status_for(&next, &primary_token).unwrap().persona_version, 4);
    assert_eq!(status_for(&next, &b_token).unwrap().persona_version, 3);
    assert_eq!(status_for(&next, &c_token).unwrap().persona_version, 3);
    assert_eq!(next.additional_owner_personas, old.additional_owner_personas);
}

#[test]
fn concurrent_owner_b_and_c_corrections_do_not_clobber_each_other() {
    let (record, primary_token, b_token, c_token) = three_owners();
    let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!(
        "vpr-rt1-canonical-tenants-{}-{suffix}.json",
        std::process::id()
    ));
    let initial = serde_json::to_vec_pretty(&record).unwrap();
    crate::persona_file_io::commit_raw(&path, &initial).unwrap();

    let mut workers = Vec::new();
    for (owner_token, persona_id) in [(b_token.clone(), "persona-b"), (c_token.clone(), "persona-c")] {
        let path = path.clone();
        workers.push(std::thread::spawn(move || {
            let _tx = persona_transaction::begin_for_path(&path).unwrap();
            let raw = crate::persona_file_io::read_raw(&path).unwrap().unwrap();
            let mut record: PersistedPersonaV2 = serde_json::from_str(&raw).unwrap();
            correction_in_record(&mut record, &owner_token, reviewed(persona_id, 4)).unwrap();
            let raw = serde_json::to_vec_pretty(&record).unwrap();
            crate::persona_file_io::commit_raw(&path, &raw).unwrap();
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    let raw = crate::persona_file_io::read_raw(&path).unwrap().unwrap();
    let final_record: PersistedPersonaV2 = serde_json::from_str(&raw).unwrap();
    final_record.clone().validate().unwrap();
    assert_eq!(status_for(&final_record, &primary_token).unwrap().persona_version, 3);
    assert_eq!(status_for(&final_record, &b_token).unwrap().persona_version, 4);
    assert_eq!(status_for(&final_record, &c_token).unwrap().persona_version, 4);
    std::fs::remove_file(&path).unwrap();
    let mut lock_name = path.into_os_string();
    lock_name.push(".transaction.lock");
    let _ = std::fs::remove_file(lock_name);
}

#[test]
fn swapped_owner_map_key_is_rejected_before_disk_commit() {
    let (mut record, _, _, _) = three_owners();
    let (key, data) = record.additional_owner_personas.pop_first().unwrap();
    record.additional_owner_personas.insert(format!("untrusted-{key}"), data);
    assert!(record.validate().is_err());
}
