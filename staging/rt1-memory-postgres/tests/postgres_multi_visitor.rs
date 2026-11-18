//! Destructive tests. Use an isolated disposable PostgreSQL database only.
use vpr_rt1_text_memory_postgres::{TextMemory,MemoryError,MemorySource};
#[test]
fn two_visitors_have_isolated_text_and_revoke() {
    let dsn=std::env::var("VPR_TEST_POSTGRES_DSN")
        .expect("VPR_TEST_POSTGRES_DSN must point to disposable PostgreSQL; never production");
    let db=TextMemory::connect(&dsn).unwrap();
    let alice=db.enroll_authorized("isolated","persona-test").unwrap();
    let bob=db.enroll_authorized("isolated","persona-test").unwrap();
    let a=db.authenticate(&alice.token).unwrap();
    let b=db.authenticate(&bob.token).unwrap();
    assert!(a.matches_scope("isolated","persona-test"));
    assert!(!a.matches_scope("other-tenant","persona-test"));
    db.set_consent(&a,true).unwrap();
    db.set_consent(&b,true).unwrap();
    db.remember(&a,"alice-01","pet","у меня собака Лада",MemorySource::VisitorExplicit,90).unwrap();
    assert_eq!(db.recall(&a,"собака",5).unwrap().facts.len(),1);
    assert!(db.recall(&b,"собака",5).unwrap().facts.is_empty());
    // Re-authentication is exactly the cross-session persistent identity path.
    let a_again=db.authenticate(&alice.token).unwrap();
    assert_eq!(db.recall(&a_again,"собака",5).unwrap().facts.len(),1);
    let rotated = db.rotate_credential(&alice.token).unwrap();
    assert!(db.authenticate(&alice.token).is_err(), "previous secret must be invalid");
    let rotated_principal = db.authenticate(&rotated).unwrap();
    assert_eq!(db.recall(&rotated_principal, "собака", 5).unwrap().facts.len(), 1);
    assert!(matches!(db.set_consent(&a,false),Err(MemoryError::NotConsented)));
    db.set_consent(&rotated_principal,false).unwrap();
    assert!(matches!(db.recall(&a_again,"собака",5),Err(MemoryError::NotConsented)));
    assert!(db.recall(&b,"",5).unwrap().facts.is_empty());
    db.revoke_credential(&rotated).unwrap();
    db.revoke_credential(&bob.token).unwrap();
    assert!(db.authenticate(&alice.token).is_err());
}
