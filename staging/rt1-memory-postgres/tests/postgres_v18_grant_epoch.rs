//! Real PostgreSQL acceptance: test DB and all five migrations are mandatory.
use vpr_rt1_text_memory_postgres::{MemoryError, TextMemory};

#[test]
#[ignore = "requires dedicated isolated PostgreSQL integration runner"]
fn queued_facts_survive_own_content_revision_but_not_new_consent_grant() {
    let dsn =
        std::env::var("VPR_TEST_POSTGRES_DSN").expect("isolated VPR_TEST_POSTGRES_DSN required");
    let db = TextMemory::connect(&dsn).expect("test PostgreSQL");
    db.check_schema().expect("five migrations required");
    let workspace = format!("v18_{}", std::process::id());
    let credential = db.enroll_authorized(&workspace, "persona").unwrap();
    let p = db.authenticate(&credential.token).unwrap();
    db.set_consent(&p, true).unwrap();
    for (id, key, value) in [
        ("batch_a", "first", "Я люблю шахматы"),
        ("batch_b", "second", "Я живу в Москве"),
        ("batch_c", "third", "У меня есть собака"),
    ] {
        db.enqueue_explicit_fact(&p, id, key, value, 30).unwrap();
    }
    assert_eq!(db.drain_scope_outbox(&p, 8).unwrap(), 3);
    assert_eq!(db.recall(&p, "", 8).unwrap().facts.len(), 3);
    // The old consent grant is destroyed, not silently replayed if enabled again.
    db.enqueue_explicit_fact(&p, "obsolete", "old", "Старая заметка", 30)
        .unwrap();
    db.set_consent(&p, false).unwrap();
    db.set_consent(&p, true).unwrap();
    assert_eq!(db.drain_scope_outbox(&p, 8).unwrap(), 0);
    assert!(db.recall(&p, "", 8).unwrap().facts.is_empty());
    db.revoke_credential(&credential.token).unwrap();
}

#[test]
#[ignore = "requires dedicated isolated PostgreSQL integration runner"]
fn revoked_consent_blocks_outbox_write() {
    let dsn =
        std::env::var("VPR_TEST_POSTGRES_DSN").expect("isolated VPR_TEST_POSTGRES_DSN required");
    let db = TextMemory::connect(&dsn).unwrap();
    let workspace = format!("v18_revoke_{}", std::process::id());
    let credential = db.enroll_authorized(&workspace, "persona").unwrap();
    let p = db.authenticate(&credential.token).unwrap();
    db.set_consent(&p, true).unwrap();
    db.enqueue_explicit_fact(&p, "pending", "item", "Личная заметка", 30)
        .unwrap();
    db.set_consent(&p, false).unwrap();
    assert!(matches!(
        db.drain_scope_outbox(&p, 8),
        Err(MemoryError::NotConsented)
    ));
    db.revoke_credential(&credential.token).unwrap();
}
