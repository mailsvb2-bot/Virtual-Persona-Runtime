//! This test requires a separately provisioned disposable PostgreSQL DB with migrations 1..5.
use vpr_rt1_text_memory_postgres::TextMemory;
#[test]
#[ignore = "requires dedicated isolated PostgreSQL integration runner"]
fn queue_insert_and_drain_on_real_postgres() {
    let dsn = std::env::var("VPR_TEST_POSTGRES_DSN").expect("isolated DB required");
    let db = TextMemory::connect(&dsn).expect("PostgreSQL connection");
    db.check_schema().expect("five migrations required");
    let workspace = format!(
        "v19_{}_{}",
        std::process::id(),
        std::thread::current()
            .name()
            .unwrap_or("test")
            .replace(' ', "_")
    );
    let credential = db.enroll_authorized(&workspace, "persona").unwrap();
    let principal = db.authenticate(&credential.token).unwrap();
    db.set_consent(&principal, true).unwrap();
    db.enqueue_explicit_fact(&principal, "queue1", "key1", "Запомни: люблю чай", 1)
        .unwrap();
    assert_eq!(db.drain_scope_outbox(&principal, 1).unwrap(), 1);
    assert_eq!(db.recall(&principal, "", 8).unwrap().facts.len(), 1);
    db.revoke_credential(&credential.token).unwrap();
}
