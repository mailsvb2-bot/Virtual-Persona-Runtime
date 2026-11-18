//! Real PostgreSQL acceptance of the same text-memory state changes used by RT1.
//! This does not claim browser signaling, speech or GPU renderer acceptance.
//! Run only against an isolated disposable database with migrations 0001..0005.
use std::{sync::Arc, thread, time::Duration};
use vpr_rt1_text_memory_postgres::{
    MemoryError, MemorySource, TextMemory,
    egress::{MemoryEgressFence, StopResult},
    orchestrator::MemoryOrchestrator,
};

#[test]
#[ignore = "requires isolated disposable PostgreSQL and all five migrations"]
fn visitor_consent_restart_correct_forget_and_revoke() {
    let dsn = std::env::var("VPR_TEST_POSTGRES_DSN").expect("isolated test DSN");
    let workspace = format!(
        "accept_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    );
    let first = TextMemory::connect(&dsn).expect("initial database connection");
    first.check_schema().expect("all five migrations");
    let credential = first
        .enroll_authorized(&workspace, "reviewed_persona")
        .expect("issue personal visitor credential");
    let principal = first.authenticate(&credential.token).expect("authenticate");
    assert!(
        matches!(
            first.recall(&principal, "", 8),
            Err(MemoryError::NotConsented)
        ),
        "enrollment alone must never authorize memory reads"
    );
    first
        .set_consent(&principal, true)
        .expect("explicit consent");
    first
        .remember(
            &principal,
            "note_first",
            "preference",
            "Люблю зелёный чай",
            MemorySource::VisitorExplicit,
            30,
        )
        .expect("durable visitor note");

    // Simulate a second visitor session after process/connection restart.
    drop(first);
    let reopened = Arc::new(TextMemory::connect(&dsn).expect("reopened DB"));
    let same_visitor = reopened
        .authenticate(&credential.token)
        .expect("stable visitor across sessions");
    let orchestrator = MemoryOrchestrator::new(Arc::clone(&reopened));
    let prepared = orchestrator
        .prepare(&same_visitor, "Что ты обо мне помнишь?")
        .expect("memory on next turn");
    assert!(prepared.input.contains("Люблю зелёный чай"));
    let lease = orchestrator
        .authorize_egress(&same_visitor, &prepared)
        .expect("authorize prepared context");
    drop(lease);

    reopened
        .remember(
            &same_visitor,
            "note_corrected",
            "preference",
            "Люблю чёрный кофе",
            MemorySource::VisitorExplicit,
            30,
        )
        .expect("correct existing fact");
    assert!(
        matches!(
            reopened.validate_revision(&same_visitor, prepared.consent_revision),
            Err(MemoryError::NotConsented)
        ),
        "previously prepared private context cannot be sent after correction"
    );
    let corrected = orchestrator
        .prepare(&same_visitor, "Что ты обо мне помнишь?")
        .expect("updated context");
    assert!(corrected.input.contains("Люблю чёрный кофе"));
    assert!(!corrected.input.contains("Люблю зелёный чай"));

    reopened
        .forget(&same_visitor, "forget_preference", "preference")
        .expect("forget requested fact");
    let after_forget = orchestrator
        .prepare(&same_visitor, "Что ты обо мне помнишь?")
        .expect("fresh no-memory turn");
    assert!(!after_forget.input.contains("Люблю чёрный кофе"));

    reopened
        .set_consent(&same_visitor, false)
        .expect("disable consent");
    assert!(matches!(
        reopened.validate_revision(&same_visitor, after_forget.consent_revision),
        Err(MemoryError::NotConsented)
    ));
    reopened
        .set_consent(&same_visitor, true)
        .expect("renew consent");
    assert!(
        reopened
            .recall(&same_visitor, "", 8)
            .expect("empty recall")
            .facts
            .is_empty()
    );
    reopened
        .revoke_credential(&credential.token)
        .expect("revoke personal key");
    assert!(matches!(
        reopened.authenticate(&credential.token),
        Err(MemoryError::NotConsented)
    ));
}

#[test]
fn stop_blocks_future_egress_even_if_provider_lease_is_in_flight() {
    let fence = Arc::new(MemoryEgressFence::new());
    let in_flight = fence.admit_owned(3).expect("permit before revoke");
    let revoke_fence = Arc::clone(&fence);
    let result = thread::spawn(move || revoke_fence.revoke(Duration::from_millis(40)))
        .join()
        .expect("STOP thread");
    assert_eq!(result, StopResult::StillInFlight);
    assert!(
        fence.admit_owned(3).is_none(),
        "STOP must block new egress immediately"
    );
    drop(in_flight);
    assert_eq!(fence.revoke(Duration::from_millis(40)), StopResult::Drained);
    assert!(
        fence.admit_owned(4).is_none(),
        "revocation must remain permanent"
    );
}
