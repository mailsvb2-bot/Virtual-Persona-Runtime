//! Requires isolated VPR_TEST_POSTGRES_DSN and schema 1..4.
//! Never use an application/production database.
use vpr_rt1_text_memory_postgres::{MemoryError, MemorySource, TextMemory};
#[test]
#[ignore = "requires dedicated isolated PostgreSQL integration runner"]
fn cache_revision_and_queue_idempotence() {
    let Ok(dsn) = std::env::var("VPR_TEST_POSTGRES_DSN") else {
        return;
    };
    let db = TextMemory::connect(&dsn).expect("isolated test database");
    db.check_schema().unwrap();
    let cred = db
        .enroll_authorized(&format!("v17_{}", std::process::id()), "persona")
        .unwrap();
    let principal = db.authenticate(&cred.token).unwrap();
    db.set_consent(&principal, true).unwrap();
    let (before, _) = db.memory_state_epoch(&principal).unwrap();
    db.remember(
        &principal,
        "v17_remember",
        "lunch",
        "Я люблю суп",
        MemorySource::VisitorExplicit,
        30,
    )
    .unwrap();
    let (after, _) = db.memory_state_epoch(&principal).unwrap();
    assert!(
        after > before,
        "normal writes must invalidate the context generation"
    );
    let queued = db
        .enqueue_explicit_fact(&principal, "v17_queue", "hobby", "Я люблю шахматы", 30)
        .unwrap();
    assert!(queued.durable);
    assert!(
        db.enqueue_explicit_fact(&principal, "v17_queue", "hobby", "Я люблю шахматы", 30)
            .unwrap()
            .durable
    );
    assert_eq!(db.drain_scope_outbox(&principal, 8).unwrap(), 1);
    assert!(
        db.recall_hybrid(&principal, "шахматы", 8)
            .unwrap()
            .iter()
            .any(|f| f.content.contains("шахматы"))
    );
    assert!(
        db.remember_reviewed(&principal, "v17_same", "lunch", "Я люблю суп", 30)
            .is_ok()
    );
    assert!(matches!(
        db.remember_reviewed(&principal, "v17_same", "different", "Другой факт", 30),
        Err(MemoryError::Conflict)
    ));
    db.revoke_credential(&cred.token).unwrap();
}
