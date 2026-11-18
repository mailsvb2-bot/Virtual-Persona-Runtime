//! Execute with VPR_TEST_POSTGRES_DSN against an isolated PostgreSQL database.
//! Dedicated ownership must deny a second application instance until the first exits.
use vpr_rt1_text_memory_postgres::{MemoryError, TextMemory};

#[test]
#[ignore = "requires dedicated isolated PostgreSQL integration runner"]
fn memory_process_owner_is_exclusive_and_released_on_drop() {
    let dsn = std::env::var("VPR_TEST_POSTGRES_DSN")
        .expect("VPR_TEST_POSTGRES_DSN must point at an isolated PostgreSQL test DB");
    let first = TextMemory::connect_exclusive(&dsn).expect("first app instance owns memory");
    assert!(matches!(
        TextMemory::connect_exclusive(&dsn),
        Err(MemoryError::Conflict)
    ));
    drop(first);
    let second = TextMemory::connect_exclusive(&dsn)
        .expect("lease must release on app instance termination");
    drop(second);
}
