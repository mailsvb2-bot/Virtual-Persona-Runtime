//! Canonical Persona persistence regressions; keeps production module bounded.
use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn revision(
    revision: u64,
    statement: &str,
    kind: &str,
    verification: &str,
) -> DurableOwnerClaimRevisionSnapshot {
    DurableOwnerClaimRevisionSnapshot {
        revision,
        statement: statement.into(),
        kind: kind.into(),
        source: "owner".into(),
        verification: verification.into(),
        derivation: "direct".into(),
    }
}

fn sample() -> DurableReviewedOwnerContextSnapshot {
    DurableReviewedOwnerContextSnapshot {
        persona_id: "persisted-owner".into(),
        persona_version: 3,
        claims: vec![DurableReviewedOwnerClaimSnapshot {
            claim_id: "preference-communication-style".into(),
            history_complete: true,
            revisions: vec![
                revision(1, "Кратко", "preference", "unverified"),
                revision(2, "Кратко", "preference", "owner_verified"),
                revision(3, "Кратко и по существу", "preference", "owner_verified"),
            ],
        }],
    }
}

#[test]
fn v1_migration_preserves_only_known_revision_and_becomes_idempotent_v2() {
    let raw = serde_json::json!({
        "schema_version": STORE_SCHEMA_V1,
        "snapshot": {
            "persona_id": "legacy-owner",
            "persona_version": 7,
            "claims": [{
                "claim_id": "legacy-opinion",
                "statement": "Единственное известное значение",
                "kind": "opinion",
                "revision": 7
            }]
        }
    })
    .to_string();

    let migrated = decode_persisted(&raw).unwrap();
    assert!(migrated.migrated_legacy);
    assert!(!migrated.snapshot.claims[0].history_complete);
    assert_eq!(migrated.snapshot.claims[0].revisions.len(), 1);
    assert_eq!(migrated.snapshot.claims[0].revisions[0].revision, 7);
    assert_eq!(
        migrated.snapshot.claims[0].revisions[0].statement,
        "Единственное известное значение"
    );

    let v2 =
        serde_json::to_string(&PersistedPersonaV2::new(migrated.snapshot.clone())).unwrap();
    let decoded_again = decode_persisted(&v2).unwrap();
    assert!(!decoded_again.migrated_legacy);
    assert_eq!(decoded_again.snapshot, migrated.snapshot);
}

#[test]
fn explicit_file_load_migrates_v1_store_once() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("vpr-persona-v1-migrate-{unique}.json"));
    let raw = serde_json::json!({
        "schema_version": STORE_SCHEMA_V1,
        "snapshot": {
            "persona_id": "legacy-owner",
            "persona_version": 4,
            "claims": [{
                "claim_id": "legacy-claim",
                "statement": "Legacy current",
                "kind": "factual",
                "revision": 4
            }]
        }
    });
    fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();

    let first = load_file(&path).unwrap().unwrap();
    assert!(!first.claims[0].history_complete);
    let persisted_after = fs::read_to_string(&path).unwrap();
    let probe: SchemaProbe = serde_json::from_str(&persisted_after).unwrap();
    assert_eq!(probe.schema_version, STORE_SCHEMA_V2);
    let second = load_file(&path).unwrap().unwrap();
    assert_eq!(second, first);

    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("tmp"));
}

#[test]
fn invalid_v1_is_not_rewritten_as_v2() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("vpr-persona-v1-invalid-{unique}.json"));
    let raw = serde_json::json!({
        "schema_version": STORE_SCHEMA_V1,
        "snapshot": {
            "persona_id": "legacy-owner",
            "persona_version": 4,
            "claims": [{
                "claim_id": "legacy-claim",
                "statement": "Legacy current",
                "kind": "not-a-real-kind",
                "revision": 4
            }]
        }
    });
    fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();

    assert!(load_file(&path).is_err());
    let unchanged = fs::read_to_string(&path).unwrap();
    let probe: SchemaProbe = serde_json::from_str(&unchanged).unwrap();
    assert_eq!(probe.schema_version, STORE_SCHEMA_V1);

    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("tmp"));
}
#[cfg(windows)]
#[test]
fn windows_chunked_store_round_trips_reviewed_persona_beyond_single_blob_limit() {
    use crate::windows_secure_store::ChunkedCredentialStore;

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let prefix = format!(
        "owner-lab-reviewed-persona-test-{}-{unique}",
        std::process::id()
    );
    let prefix: &'static str = Box::leak(prefix.into_boxed_str());
    let store = ChunkedCredentialStore::new("Virtual-Persona-Runtime-Test", prefix, None);
    let _ = store.delete();

    let mut large = sample();
    large.claims[0].revisions[2].statement = "Ж".repeat(4_000);
    let raw = serde_json::to_string(&PersistedPersonaV2::new(large.clone()))
        .expect("sample must serialize");
    assert!(raw.len() > 2_560);

    store
        .save(&raw)
        .expect("chunked Credential Manager store must accept large Persona");
    let restored_raw = store
        .load()
        .expect("chunked Credential Manager store must read Persona")
        .expect("stored Persona must exist");
    let restored: PersistedPersonaV2 =
        serde_json::from_str(&restored_raw).expect("stored Persona must decode");
    assert_eq!(restored.validate().unwrap(), large);
    store.delete().unwrap();
}

#[test]
fn failed_staging_never_truncates_committed_persona() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!("vpr-persona-atomic-{unique}"));
    fs::create_dir_all(&base).unwrap();
    let path = base.join("reviewed.json");
    let original = sample();
    save_file(&path, &original).unwrap();
    let raw = fs::read(&path).unwrap();

    // A directory cannot be atomically replaced by a file. The committed
    // Persona must remain unchanged after this unrelated commit failure.
    let directory_target = base.join("directory-target");
    fs::create_dir(&directory_target).unwrap();
    assert!(save_file(&directory_target, &original).is_err());
    assert_eq!(fs::read(&path).unwrap(), raw);
    assert_eq!(load_file(&path).unwrap(), Some(original));
    let remaining: Vec<_> = fs::read_dir(&base)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(remaining.len(), 4, "only committed files and stable transaction locks may remain");
    fs::remove_file(&path).unwrap();
    fs::remove_file(base.join("reviewed.json.transaction.lock")).unwrap();
    fs::remove_file(base.join("directory-target.transaction.lock")).unwrap();
    fs::remove_dir(&directory_target).unwrap();
    fs::remove_dir(&base).unwrap();
}

#[cfg(unix)]
#[test]
fn private_reviewed_file_permissions_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!("vpr-persona-private-{unique}"));
    fs::create_dir_all(&base).unwrap();
    let path = base.join("reviewed.json");
    save_file(&path, &sample()).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_file(&path).unwrap();
    fs::remove_file(base.join("reviewed.json.transaction.lock")).unwrap();
    fs::remove_dir(&base).unwrap();
}

#[test]
fn concurrent_same_version_reviewed_write_must_not_replace_canonical_history() {
    let base = std::env::temp_dir().join(format!(
        "vpr-persona-conflict-{}-{}", std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&base).unwrap();
    let path = base.join("reviewed.json");
    let first = sample();
    save_file(&path, &first).unwrap();
    let mut competing = first.clone();
    competing.claims[0].revisions[2].statement = "Conflicting revision".into();
    assert!(save_file(&path, &competing).is_err());
    assert_eq!(load_file(&path).unwrap(), Some(first));
    fs::remove_file(&path).unwrap();
    fs::remove_file(base.join("reviewed.json.transaction.lock")).unwrap();
    fs::remove_dir(&base).unwrap();
}

#[test]
fn explicit_file_store_round_trips_and_replaces_exact_history() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("vpr-persona-store-{unique}.json"));
    let first = sample();
    save_file(&path, &first).unwrap();
    assert_eq!(load_file(&path).unwrap(), Some(first.clone()));

    let mut second = first;
    second.persona_version = 4;
    second.claims[0].revisions.push(revision(
        4,
        "Обновлённый снимок",
        "opinion",
        "owner_verified",
    ));
    save_file(&path, &second).unwrap();
    let restored = load_file(&path).unwrap().unwrap();
    assert_eq!(restored, second);
    assert_eq!(restored.claims[0].revisions.len(), 4);
    assert_eq!(
        restored.claims[0].revisions[2].statement,
        "Кратко и по существу"
    );

    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("tmp"));
}
