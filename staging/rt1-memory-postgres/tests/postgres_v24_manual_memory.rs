//! Isolated PostgreSQL regression of the canonical method called by HTTP.
//! Requires migrations 0001..0005 and VPR_TEST_POSTGRES_DSN (never production).
use vpr_rt1_text_memory_postgres::{TextMemory, advanced::ConsolidationResult};

#[test]
#[ignore = "requires disposable TLS PostgreSQL with migrations 0001..0005"]
fn button_write_and_correction_are_versioned_and_idempotent() {
    let dsn = std::env::var("VPR_TEST_POSTGRES_DSN").expect("isolated PostgreSQL DSN");
    let db = TextMemory::connect(&dsn).expect("connect");
    db.check_schema().expect("schema");
    let namespace = format!(
        "manual_v24_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let key = db.enroll_authorized(&namespace, "persona").expect("enroll");
    let subject = db.authenticate(&key.token).expect("authenticate");
    db.set_consent(&subject, true).expect("consent");

    assert_eq!(
        db.remember_reviewed(&subject, "button_op_1", "preference", "Люблю чай", 365)
            .expect("initial write"),
        ConsolidationResult::Inserted
    );
    // Retry with the same operation is not a second mutation.
    let retry = db
        .remember_reviewed(&subject, "button_op_1", "preference", "Люблю чай", 365)
        .expect("idempotent retry");
    assert!(matches!(retry, ConsolidationResult::Unchanged));
    assert_eq!(
        db.remember_reviewed(&subject, "button_op_2", "preference", "Люблю кофе", 365)
            .expect("correction"),
        ConsolidationResult::Corrected
    );
    let facts = db.list_facts(&subject, None, 50).expect("read facts");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].content, "Люблю кофе");
    db.erase_complete(&subject, 30).expect("erase");
    db.revoke_credential(&key.token).expect("revoke");
}
