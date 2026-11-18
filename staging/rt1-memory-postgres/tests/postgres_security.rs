//! Run with real isolated PostgreSQL and all three migrations applied.
//! This test is intentionally NOT skippable.
use vpr_rt1_text_memory_postgres::{TextMemory,MemoryError,MemorySource};
#[test]
fn rotation_and_consent_revoke_are_durable() {
    let dsn=std::env::var("VPR_TEST_POSTGRES_DSN").expect("isolated test postgres DSN required");
    let m=TextMemory::connect(&dsn).unwrap();
    let c=m.enroll_authorized("security-suite","persona").unwrap();
    let p=m.authenticate(&c.token).unwrap();
    m.set_consent(&p,true).unwrap();
    m.remember(&p,"op-1","language","Пользователь любит русский язык",MemorySource::VisitorExplicit,10).unwrap();
    let old=m.recall(&p,"язык",5).unwrap();
    assert_eq!(old.facts.len(),1);
    let new_token=m.rotate_credential(&c.token).unwrap();
    assert!(matches!(m.authenticate(&c.token),Err(MemoryError::NotConsented)));
    let new_p=m.authenticate(&new_token).unwrap();
    assert_ne!(new_p,p);
    // These are the dangerous stale references retained by already-running
    // conversations. They must fail even when consent is still enabled.
    assert!(matches!(m.recall(&p,"язык",5),Err(MemoryError::NotConsented)));
    assert!(matches!(m.remember(&p,"op-2","stale","denied",MemorySource::VisitorExplicit,10),Err(MemoryError::NotConsented)));
    assert!(matches!(m.set_consent(&p,false),Err(MemoryError::NotConsented)));
    m.set_consent(&new_p,false).unwrap();
    assert!(matches!(m.validate_revision(&p,old.consent_revision),Err(MemoryError::NotConsented)));
    m.set_consent(&new_p,true).unwrap();
    assert!(m.recall(&new_p,"",5).unwrap().facts.is_empty());
    m.revoke_credential(&new_token).unwrap();
    assert!(matches!(m.authenticate(&new_token),Err(MemoryError::NotConsented)));
}
