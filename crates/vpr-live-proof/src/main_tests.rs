use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{BoundaryError, atomic_write};

fn temp_path(suffix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "vpr-live-proof-write-{}-{nanos}-{suffix}",
        std::process::id()
    ))
}

#[test]
fn atomic_write_publishes_new_file() {
    let path = temp_path("new.json");
    atomic_write(&path, b"candidate").expect("new artifact should publish");
    assert_eq!(fs::read(&path).expect("published artifact"), b"candidate");
    let _ = fs::remove_file(path);
}

#[test]
fn atomic_write_never_overwrites_existing_artifact() {
    let path = temp_path("existing.json");
    fs::write(&path, b"immutable").expect("sentinel");
    assert_eq!(
        atomic_write(&path, b"replacement"),
        Err(BoundaryError::ArtifactWriteFailed)
    );
    assert_eq!(fs::read(&path).expect("sentinel remains"), b"immutable");
    let _ = fs::remove_file(path);
}
