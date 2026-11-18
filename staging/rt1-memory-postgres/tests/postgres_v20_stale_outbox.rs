//! Requires isolated PostgreSQL and migrations 1..5; never point at production.
use vpr_rt1_text_memory_postgres::{TextMemory,MemorySource};
#[test]
fn direct_correction_or_erase_cannot_replay_queued_old_text() {
    let dsn=std::env::var("VPR_TEST_POSTGRES_DSN").expect("isolated DB required");
    let db=TextMemory::connect(&dsn).expect("connect");
    db.check_schema().expect("schema");
    let workspace=format!("v20_stale_{}",std::process::id());
    let credential=db.enroll_authorized(&workspace,"persona").expect("enroll");
    let p=db.authenticate(&credential.token).expect("auth");
    db.set_consent(&p,true).expect("consent");
    db.enqueue_explicit_fact(&p,"old1","preference","люблю старое",30).expect("queue");
    db.remember(&p,"correction1","preference","люблю новое",MemorySource::VisitorExplicit,30).expect("correct");
    assert_eq!(db.drain_scope_outbox(&p,8).expect("drain"),0);
    let facts=db.recall(&p,"",8).expect("recall").facts;
    assert_eq!(facts.len(),1);
    assert_eq!(facts[0].content,"люблю новое");
    db.enqueue_explicit_fact(&p,"same1","preference","очень старое",30).expect("queue same");
    db.remember_reviewed(&p,"same_review","preference","люблю новое",30).expect("same correction");
    assert_eq!(db.drain_scope_outbox(&p,8).expect("same drain"),0);
    db.enqueue_explicit_fact(&p,"old2","preference","старое повторно",30).expect("queue2");
    db.forget(&p,"forget1","preference").expect("forget");
    assert_eq!(db.drain_scope_outbox(&p,8).expect("drain2"),0);
    assert!(db.recall(&p,"",8).expect("recall2").facts.is_empty());
    db.revoke_credential(&credential.token).expect("revoke");
}
