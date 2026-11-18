//! Run with VPR_TEST_POSTGRES_DSN=postgresql://... (trusted disposable test DB).
//! Never point these destructive tests at production.
use vpr_rt1_text_memory_postgres::{StablePrincipal,TextMemory,MemorySource,MemoryError,context_for_llm};
#[test]
fn explicit_consent_recall_idempotency_erase() {
    let dsn=std::env::var("VPR_TEST_POSTGRES_DSN").expect("Set VPR_TEST_POSTGRES_DSN to an isolated TEST database; do not run against production");
    let memory=TextMemory::connect(&dsn).unwrap();
    let credential=memory.enroll_authorized("integration", "persona").unwrap();
    let p=memory.authenticate(&credential.token).unwrap();
    let _=memory.erase_visitor(&p);
    assert!(matches!(memory.recall(&p,"hello",5),Err(MemoryError::NotConsented)));
    memory.set_consent(&p,true).unwrap();
    memory.remember(&p,"turn-1","note-1","люблю зелёный чай",MemorySource::VisitorExplicit,10).unwrap();
    memory.remember(&p,"turn-1","note-1","люблю зелёный чай",MemorySource::VisitorExplicit,10).unwrap();
    assert!(matches!(memory.remember(&p,"turn-1","note-1","другая запись",MemorySource::VisitorExplicit,10),Err(MemoryError::Conflict)));
    let r=memory.recall(&p,"чай",5).unwrap();
    assert_eq!(r.facts.len(),1);
    let s=context_for_llm(&r.facts).unwrap();
    assert!(s.contains("зелёный чай"));
    memory.set_consent(&p,false).unwrap();
    assert!(matches!(memory.validate_revision(&p,r.consent_revision),Err(MemoryError::NotConsented)));
    memory.set_consent(&p,true).unwrap();
    assert!(memory.recall(&p,"",5).unwrap().facts.is_empty());
    memory.erase_visitor(&p).unwrap();
    assert!(matches!(memory.recall(&p,"",5),Err(MemoryError::NotConsented)));
    memory.revoke_credential(&credential.token).unwrap();
    assert!(memory.authenticate(&credential.token).is_err());
}
