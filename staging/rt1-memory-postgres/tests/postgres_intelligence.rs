//! Opt-in actual PostgreSQL tests. Set VPR_TEST_POSTGRES_DSN and run migrations 1-4 first.
//! NEVER run against production: tests create and erase data.
use vpr_rt1_text_memory_postgres::{TextMemory,policy::fingerprint,advanced::ConsolidationResult};
#[test]fn approved_fact_history_and_durable_queue(){
    let Ok(dsn)=std::env::var("VPR_TEST_POSTGRES_DSN") else{return};
    let store=TextMemory::connect(&dsn).expect("PostgreSQL must connect");
    store.check_schema().unwrap();
    let uid=std::process::id().to_string();
    let cred=store.enroll_authorized(&format!("test_{uid}"),"persona").unwrap();
    let p=store.authenticate(&cred.token).unwrap();
    store.set_consent(&p,true).unwrap();
    let fact=store.remember_reviewed(&p,"approved_first","occupation","Работаю врачом",90).unwrap();
    assert_eq!(fact,ConsolidationResult::Inserted);
    assert_eq!(store.remember_reviewed(&p,"approved_second","occupation","Теперь работаю учителем",90).unwrap(),ConsolidationResult::Corrected);
    assert!(!store.recall_hybrid(&p,"учителем",8).unwrap().is_empty());
    let receipt=store.enqueue_explicit_fact(&p,"queued_once","preference","Люблю чай",90).unwrap();
    assert!(receipt.durable);
    assert_eq!(store.drain_scope_outbox(&p,4).unwrap(),1);
    assert_eq!(store.drain_scope_outbox(&p,4).unwrap(),0);
    assert!(!fingerprint("Люблю чай").is_empty());
    store.erase_complete(&p,30).unwrap();
    assert!(store.recall(&p,"",8).is_err());
    store.revoke_credential(&cred.token).unwrap();
}
