use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use vpr_evaluation::{RT0_MANUAL_SUPPORTING_FILES, sha256_hex};

use super::fail;

const LOCK_FILE: &str = ".rt0-supporting-capture.lock";
const RECOVERY_CLAIM_FILE: &str = ".rt0-supporting-capture.recovery";
const COMMIT_MARKER: &str = ".rt0-supporting-capture.commit";
const STAGE_PREFIX: &str = ".rt0-supporting-capture.stage.";
const STALE_LOCK_AFTER: Duration = Duration::from_secs(30);
const JOURNAL_SCHEMA: &str = "rt0-manual-supporting-capture-journal-0.1";

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct TransactionJournal {
    schema_version: String,
    candidate_sha: String,
    provider_state_sha256: String,
    input_sha256: String,
    artifact_sha256: BTreeMap<String, String>,
}

impl TransactionJournal {
    pub(super) fn new(
        candidate_sha: &str,
        provider_state_sha256: &str,
        input_bytes: &[u8],
        artifacts: &[(&'static str, Vec<u8>)],
    ) -> Self {
        Self {
            schema_version: JOURNAL_SCHEMA.into(),
            candidate_sha: candidate_sha.into(),
            provider_state_sha256: provider_state_sha256.into(),
            input_sha256: sha256_hex(input_bytes),
            artifact_sha256: artifact_digests(artifacts),
        }
    }
}

pub(super) enum LockOutcome {
    Acquired(CaptureLock),
    AlreadyCommitted,
}

pub(super) struct CaptureLock {
    path: PathBuf,
}

impl CaptureLock {
    pub(super) fn acquire(
        root: &Path,
        journal: &TransactionJournal,
        expected: &[(&'static str, String)],
        artifacts: &[(&'static str, Vec<u8>)],
    ) -> Result<LockOutcome, i32> {
        Self::acquire_with_timeout(root, journal, expected, artifacts, STALE_LOCK_AFTER)
    }

    fn acquire_with_timeout(
        root: &Path,
        journal: &TransactionJournal,
        expected: &[(&'static str, String)],
        artifacts: &[(&'static str, Vec<u8>)],
        stale_after: Duration,
    ) -> Result<LockOutcome, i32> {
        let lock_path = root.join(LOCK_FILE);
        match create_journal_file(&lock_path, journal) {
            Ok(()) => {
                let lock = Self { path: lock_path };
                recover_commit_marker_under_lock(root, journal, expected)?;
                Ok(LockOutcome::Acquired(lock))
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                recover_stale_lock(root, journal, expected, artifacts, stale_after)
            }
            Err(_) => Err(fail("CAPTURE_LOCK_FAILED")),
        }
    }
}

impl Drop for CaptureLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct RecoveryClaim {
    path: PathBuf,
}

impl RecoveryClaim {
    fn acquire(root: &Path, stale_after: Duration) -> Result<Self, i32> {
        let path = root.join(RECOVERY_CLAIM_FILE);
        for _ in 0..2 {
            match create_empty_claim(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if file_age(&path)? < stale_after {
                        return Err(fail("CAPTURE_RECOVERY_IN_PROGRESS"));
                    }
                    match fs::remove_file(&path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(_) => return Err(fail("CAPTURE_RECOVERY_CLAIM_FAILED")),
                    }
                }
                Err(_) => return Err(fail("CAPTURE_RECOVERY_CLAIM_FAILED")),
            }
        }
        Err(fail("CAPTURE_RECOVERY_IN_PROGRESS"))
    }
}

impl Drop for RecoveryClaim {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn create_empty_claim(path: &Path) -> std::io::Result<()> {
    let file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.sync_all()
}

fn file_age(path: &Path) -> Result<Duration, i32> {
    let modified = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|_| fail("CAPTURE_RECOVERY_CLAIM_FAILED"))?;
    Ok(SystemTime::now().duration_since(modified).unwrap_or_default())
}

fn recover_commit_marker_under_lock(
    root: &Path,
    journal: &TransactionJournal,
    expected: &[(&'static str, String)],
) -> Result<(), i32> {
    let marker = root.join(COMMIT_MARKER);
    if !marker.exists() {
        return Ok(());
    }
    let marker_journal = read_journal(&marker)?;
    if marker_journal != *journal {
        return Err(fail("STALE_CAPTURE_MISMATCH"));
    }
    if !restore_scaffold(root, expected) {
        return Err(fail("STALE_CAPTURE_RECOVERY_FAILED"));
    }
    cleanup_staged_files(root);
    fs::remove_file(marker).map_err(|_| fail("STALE_CAPTURE_RECOVERY_FAILED"))
}

fn recover_stale_lock(
    root: &Path,
    journal: &TransactionJournal,
    expected: &[(&'static str, String)],
    artifacts: &[(&'static str, Vec<u8>)],
    stale_after: Duration,
) -> Result<LockOutcome, i32> {
    let lock_path = root.join(LOCK_FILE);
    if file_age(&lock_path)? < stale_after {
        return Err(fail("CAPTURE_IN_PROGRESS"));
    }

    let recovery_claim = RecoveryClaim::acquire(root, stale_after)?;
    if !lock_path.exists() {
        drop(recovery_claim);
        return CaptureLock::acquire_with_timeout(
            root,
            journal,
            expected,
            artifacts,
            stale_after,
        );
    }
    if file_age(&lock_path)? < stale_after {
        return Err(fail("CAPTURE_IN_PROGRESS"));
    }

    let stale = read_journal(&lock_path)?;
    if stale != *journal {
        return Err(fail("STALE_CAPTURE_MISMATCH"));
    }

    let marker = root.join(COMMIT_MARKER);
    if marker.exists() {
        let marker_journal = read_journal(&marker)?;
        if marker_journal != *journal || !restore_scaffold(root, expected) {
            return Err(fail("STALE_CAPTURE_RECOVERY_FAILED"));
        }
        cleanup_staged_files(root);
        fs::remove_file(&marker).map_err(|_| fail("STALE_CAPTURE_RECOVERY_FAILED"))?;
    } else if artifacts_match(root, artifacts) {
        cleanup_staged_files(root);
        fs::remove_file(&lock_path).map_err(|_| fail("STALE_CAPTURE_RECOVERY_FAILED"))?;
        drop(recovery_claim);
        return Ok(LockOutcome::AlreadyCommitted);
    } else if !scaffold_matches(root, expected) {
        return Err(fail("STALE_CAPTURE_AMBIGUOUS"));
    }

    cleanup_staged_files(root);
    fs::remove_file(&lock_path).map_err(|_| fail("STALE_CAPTURE_RECOVERY_FAILED"))?;
    drop(recovery_claim);
    CaptureLock::acquire_with_timeout(root, journal, expected, artifacts, stale_after)
}

pub(super) fn artifact_digests(artifacts: &[(&'static str, Vec<u8>)]) -> BTreeMap<String, String> {
    artifacts
        .iter()
        .map(|(name, bytes)| ((*name).to_owned(), sha256_hex(bytes)))
        .collect()
}

pub(super) fn write_artifacts_transactional(
    root: &Path,
    expected: &[(&'static str, String)],
    artifacts: &[(&'static str, Vec<u8>)],
    journal: &TransactionJournal,
) -> Result<(), i32> {
    let nonce = transaction_nonce();
    let mut staged = Vec::with_capacity(artifacts.len());
    for (index, (name, bytes)) in artifacts.iter().enumerate() {
        if !RT0_MANUAL_SUPPORTING_FILES.contains(name) {
            cleanup_paths(&staged);
            return Err(fail("INTERNAL_ERROR"));
        }
        let path = root.join(format!(
            "{STAGE_PREFIX}{}.{}.{}.tmp",
            process::id(),
            nonce,
            index
        ));
        if let Err(code) = write_new_synced(&path, bytes) {
            cleanup_paths(&staged);
            return Err(code);
        }
        staged.push(path);
    }

    let marker = root.join(COMMIT_MARKER);
    if let Err(error) = create_journal_file(&marker, journal) {
        cleanup_paths(&staged);
        return Err(fail(if error.kind() == std::io::ErrorKind::AlreadyExists {
            "CAPTURE_IN_PROGRESS"
        } else {
            "OUTPUT_STAGE_FAILED"
        }));
    }

    for (replaced, ((name, _), staged_path)) in artifacts.iter().zip(&staged).enumerate() {
        let Ok(bytes) = fs::read(staged_path) else {
            let rollback_ok = restore_scaffold(root, &expected[..replaced]);
            cleanup_paths(&staged);
            if rollback_ok {
                let _ = fs::remove_file(&marker);
            }
            return Err(fail(if rollback_ok {
                "OUTPUT_STAGE_FAILED"
            } else {
                "OUTPUT_ROLLBACK_FAILED"
            }));
        };
        if overwrite_synced(&root.join(name), &bytes).is_err() {
            let rollback_ok = restore_scaffold(root, &expected[..=replaced]);
            cleanup_paths(&staged);
            if rollback_ok {
                let _ = fs::remove_file(&marker);
            }
            return Err(fail(if rollback_ok {
                "OUTPUT_WRITE_FAILED"
            } else {
                "OUTPUT_ROLLBACK_FAILED"
            }));
        }
    }

    cleanup_paths(&staged);
    fs::remove_file(marker).map_err(|_| fail("OUTPUT_COMMIT_FINALIZE_FAILED"))
}

fn create_journal_file(path: &Path, journal: &TransactionJournal) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(journal).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("journal path has no parent"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("journal path has no file name"))?
        .to_string_lossy();
    let temp = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        process::id(),
        transaction_nonce()
    ));

    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::hard_link(&temp, path)?;
        Ok(())
    })();
    let _ = fs::remove_file(&temp);
    result
}

fn read_journal(path: &Path) -> Result<TransactionJournal, i32> {
    let bytes = fs::read(path).map_err(|_| fail("STALE_CAPTURE_JOURNAL_INVALID"))?;
    serde_json::from_slice(&bytes).map_err(|_| fail("STALE_CAPTURE_JOURNAL_INVALID"))
}

fn artifacts_match(root: &Path, artifacts: &[(&'static str, Vec<u8>)]) -> bool {
    artifacts
        .iter()
        .all(|(name, bytes)| fs::read(root.join(name)).is_ok_and(|actual| actual == *bytes))
}

fn scaffold_matches(root: &Path, expected: &[(&'static str, String)]) -> bool {
    expected.iter().all(|(name, content)| {
        fs::read(root.join(name)).is_ok_and(|actual| actual == content.as_bytes())
    })
}

fn restore_scaffold(root: &Path, expected: &[(&'static str, String)]) -> bool {
    let mut restored = true;
    for (name, content) in expected {
        if restore_synced(&root.join(name), content.as_bytes()).is_err() {
            restored = false;
        }
    }
    restored
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), i32> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| fail("OUTPUT_STAGE_FAILED"))?;
    if file.write_all(bytes).is_err() || file.sync_all().is_err() {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(fail("OUTPUT_STAGE_FAILED"));
    }
    Ok(())
}

fn overwrite_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(std::io::Error::other("target is not a regular file"));
    }
    let mut file = OpenOptions::new().write(true).truncate(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn restore_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(std::io::Error::other("target is not a regular file"));
        }
        Ok(_) | Err(_) => {}
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn cleanup_paths(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn cleanup_staged_files(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(STAGE_PREFIX) && name.ends_with(".tmp") {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn transaction_nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixtures {
        expected: Vec<(&'static str, String)>,
        artifacts: Vec<(&'static str, Vec<u8>)>,
        journal: TransactionJournal,
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "vpr-supporting-capture-{name}-{}-{}",
            process::id(),
            transaction_nonce()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn fixtures() -> Fixtures {
        let candidate = "a".repeat(40);
        let provider = "b".repeat(64);
        let expected = vpr_evaluation::rt0_manual_supporting_scaffold(&candidate, &provider);
        let artifacts = expected
            .iter()
            .map(|(name, content)| (*name, format!("reviewed:{content}").into_bytes()))
            .collect::<Vec<_>>();
        let journal = TransactionJournal::new(&candidate, &provider, b"reviewed input", &artifacts);
        Fixtures {
            expected,
            artifacts,
            journal,
        }
    }

    #[test]
    fn lock_serializes_concurrent_writers() {
        let root = temp_dir("lock");
        let Fixtures {
            expected,
            artifacts,
            journal,
        } = fixtures();
        for (name, content) in &expected {
            fs::write(root.join(name), content).unwrap();
        }

        let first = match CaptureLock::acquire_with_timeout(
            &root,
            &journal,
            &expected,
            &artifacts,
            Duration::from_secs(60),
        )
        .unwrap()
        {
            LockOutcome::Acquired(lock) => lock,
            LockOutcome::AlreadyCommitted => panic!("unexpected committed recovery"),
        };
        assert_eq!(
            CaptureLock::acquire_with_timeout(
                &root,
                &journal,
                &expected,
                &artifacts,
                Duration::from_secs(60),
            )
            .err(),
            Some(2)
        );
        drop(first);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_claim_serializes_stale_recovery() {
        let root = temp_dir("recovery-claim");
        let first = RecoveryClaim::acquire(&root, Duration::from_secs(60)).unwrap();
        assert_eq!(
            RecoveryClaim::acquire(&root, Duration::from_secs(60)).err(),
            Some(2)
        );
        drop(first);
        assert!(RecoveryClaim::acquire(&root, Duration::from_secs(60)).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_mid_commit_is_restored_by_single_recovery_owner() {
        let root = temp_dir("stale-mid-commit");
        let Fixtures {
            expected,
            artifacts,
            journal,
        } = fixtures();
        for (name, content) in &expected {
            fs::write(root.join(name), content).unwrap();
        }
        create_journal_file(&root.join(LOCK_FILE), &journal).unwrap();
        create_journal_file(&root.join(COMMIT_MARKER), &journal).unwrap();
        fs::write(root.join(artifacts[0].0), &artifacts[0].1).unwrap();

        let recovered = CaptureLock::acquire_with_timeout(
            &root,
            &journal,
            &expected,
            &artifacts,
            Duration::ZERO,
        )
        .unwrap();
        assert!(matches!(recovered, LockOutcome::Acquired(_)));
        assert!(scaffold_matches(&root, &expected));
        drop(recovered);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn orphan_commit_marker_is_recovered_only_after_lock_claim() {
        let root = temp_dir("orphan-marker");
        let Fixtures {
            expected,
            artifacts,
            journal,
        } = fixtures();
        for (name, content) in &expected {
            fs::write(root.join(name), content).unwrap();
        }
        create_journal_file(&root.join(COMMIT_MARKER), &journal).unwrap();
        fs::write(root.join(artifacts[0].0), &artifacts[0].1).unwrap();

        let recovered = CaptureLock::acquire_with_timeout(
            &root,
            &journal,
            &expected,
            &artifacts,
            Duration::ZERO,
        )
        .unwrap();
        assert!(matches!(recovered, LockOutcome::Acquired(_)));
        assert!(root.join(LOCK_FILE).exists());
        assert!(!root.join(COMMIT_MARKER).exists());
        assert!(scaffold_matches(&root, &expected));
        drop(recovered);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_completed_capture_returns_without_rewriting() {
        let root = temp_dir("stale-complete");
        let Fixtures {
            expected,
            artifacts,
            journal,
        } = fixtures();
        for (name, bytes) in &artifacts {
            fs::write(root.join(name), bytes).unwrap();
        }
        create_journal_file(&root.join(LOCK_FILE), &journal).unwrap();

        let recovered = CaptureLock::acquire_with_timeout(
            &root,
            &journal,
            &expected,
            &artifacts,
            Duration::ZERO,
        )
        .unwrap();
        assert!(matches!(recovered, LockOutcome::AlreadyCommitted));
        assert!(artifacts_match(&root, &artifacts));
        assert!(!root.join(LOCK_FILE).exists());
        let _ = fs::remove_dir_all(root);
    }
}
