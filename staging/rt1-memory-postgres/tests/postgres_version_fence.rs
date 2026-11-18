//! Requires real disposable PostgreSQL with all three migrations. Never skip.
use vpr_rt1_text_memory_postgres::{MemoryError,MemorySource,TextMemory};
#[test]
fn erased_memory_does_not_revalidate_stale_context() {
    let dsn=std::env::var("VPR_TEST_POSTGRES_DSN").expect("isolated PostgreSQL required");
    let db=TextMemory::connect(&dsn).unwrap();
    let c=db.enroll_authorized("version-suite","persona").unwrap();
    let p=db.authenticate(&c.token).unwrap();
    let v1=db.set_consent(&p,true).unwrap();
    db.remember(&p,"first-operation","pref","люблю чай",MemorySource::VisitorExplicit,30).unwrap();
    assert_eq!(db.recall(&p,"чай",5).unwrap().facts.len(),1);
    db.erase_visitor(&p).unwrap();
    assert!(matches!(db.validate_revision(&p,v1),Err(MemoryError::NotConsented)));
    let v2=db.set_consent(&p,true).unwrap();
    assert!(v2>v1,"erase + reconsent must advance generation");
    assert!(matches!(db.validate_revision(&p,v1),Err(MemoryError::NotConsented)));
    assert!(db.recall(&p,"чай",5).unwrap().facts.is_empty());
    db.revoke_credential(&c.token).unwrap();
}
