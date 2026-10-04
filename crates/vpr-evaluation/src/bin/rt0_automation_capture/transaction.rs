use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use vpr_evaluation::sha256_hex;

use super::fail;

const LOCK_FILE: &str = ".rt0-automation-capture.lock";
const COMMIT_MARKER: &str = ".rt0-automation-capture.commit";
const STAGE_PREFIX: &str = ".rt0-automation-capture.stage.";
const JOURNAL_SCHEMA: &str = "rt0-automation-supporting-capture-journal-0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CaptureOutcome {
    Committed,
    AlreadyCommitted,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct TransactionJournal {
    schema_version: String,
    candidate_sha: String,
    input_sha256: String,
    ci_sha256: String,
    e2e_sha256: String,
}

impl TransactionJournal {
    fn new(
        candidate_sha: &str,
        input_bytes: &[u8],
        artifacts: &[(&'static str, Vec<u8>)],
    ) -> Result<Self, i32> {
        let digest = |name: &str| {
            artifacts
                .iter()
                .find(|(artifact_name, _)| *artifact_name == name)
                .map(|(_, bytes)| sha256_hex(bytes))
                .ok_or_else(|| fail("INTERNAL_ERROR"))
        };
        Ok(Self {
            schema_version: JOURNAL_SCHEMA.into(),
            candidate_sha: candidate_sha.into(),
            input_sha256: sha256_hex(input_bytes),
            ci_sha256: digest("ci-evidence.json")?,
            e2e_sha256: digest("e2e-evidence.json")?,
        })
    }
}

pub(super) struct AutomationCaptureTransaction<'a> {
    root: &'a Path,
    journal: TransactionJournal,
    scaffold: &'a [(&'static str, Vec<u8>)],
    artifacts: &'a [(&'static str, Vec<u8>)],
}

impl<'a> AutomationCaptureTransaction<'a> {
    #[must_use]
    pub(super) fn new(
        root: &'a Path,
        candidate_sha: &str,
        input_bytes: &[u8],
        scaffold: &'a [(&'static str, Vec<u8>)],
        artifacts: &'a [(&'static str, Vec<u8>)],
    ) -> Self {
        let journal = TransactionJournal::new(candidate_sha, input_bytes, artifacts)
            .expect("canonical automation artifact set");
        Self {
            root,
            journal,
            scaffold,
            artifacts,
        }
    }

    pub(super) fn commit(&self) -> Result<CaptureOutcome, i32> {
        let lock = acquire_lock(self.root)?;
        recover_if_needed(
            self.root,
            &self.journal,
            self.scaffold,
            self.artifacts,
        )?;

        if artifacts_match(self.root, self.artifacts) {
            return Ok(CaptureOutcome::AlreadyCommitted);
        }
        if !artifacts_match(self.root, self.scaffold) {
            return Err(fail("AUTOMATION_CAPTURE_AMBIGUOUS"));
        }

        let staged = stage_artifacts(self.root, self.artifacts)?;
        let marker = self.root.join(COMMIT_MARKER);
        if let Err(error) = create_journal(&marker, &self.journal) {
            cleanup(&staged);
            return Err(fail(if error.kind() == std::io::ErrorKind::AlreadyExists {
                "AUTOMATION_CAPTURE_IN_PROGRESS"
            } else {
                "AUTOMATION_CAPTURE_STAGE_FAILED"
            }));
        }

        let result = replace_artifacts(self.root, self.scaffold, self.artifacts, &staged);
        cleanup(&staged);
        if result.is_ok() {
            fs::remove_file(marker).map_err(|_| fail("AUTOMATION_CAPTURE_FINALIZE_FAILED"))?;
        }
        drop(lock);
        result.map(|()| CaptureOutcome::Committed)
    }
}

struct CaptureLock(File);

impl Drop for CaptureLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn acquire_lock(root: &Path) -> Result<CaptureLock, i32> {
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(root.join(LOCK_FILE))
        .map_err(|_| fail("AUTOMATION_CAPTURE_LOCK_FAILED"))?;
    FileExt::try_lock_exclusive(&file).map_err(|error| {
        fail(if error.kind() == std::io::ErrorKind::WouldBlock {
            "AUTOMATION_CAPTURE_IN_PROGRESS"
        } else {
            "AUTOMATION_CAPTURE_LOCK_FAILED"
        })
    })?;
    Ok(CaptureLock(file))
}

fn recover_if_needed(
    root: &Path,
    journal: &TransactionJournal,
    scaffold: &[(&'static str, Vec<u8>)],
    artifacts: &[(&'static str, Vec<u8>)],
) -> Result<(), i32> {
    let marker = root.join(COMMIT_MARKER);
    if !marker.exists() {
        cleanup_staged(root);
        return Ok(());
    }

    let marker_bytes =
        fs::read(&marker).map_err(|_| fail("AUTOMATION_CAPTURE_JOURNAL_INVALID"))?;
    let previous: TransactionJournal = serde_json::from_slice(&marker_bytes)
        .map_err(|_| fail("AUTOMATION_CAPTURE_JOURNAL_INVALID"))?;
    if previous != *journal {
        return Err(fail("STALE_AUTOMATION_CAPTURE_MISMATCH"));
    }

    if artifacts_match(root, artifacts) {
        cleanup_staged(root);
        fs::remove_file(marker).map_err(|_| fail("AUTOMATION_CAPTURE_RECOVERY_FAILED"))?;
        return Ok(());
    }
    restore_transaction_owned(root, scaffold, artifacts)
        .map_err(|_| fail("STALE_AUTOMATION_CAPTURE_AMBIGUOUS"))?;
    cleanup_staged(root);
    fs::remove_file(marker).map_err(|_| fail("AUTOMATION_CAPTURE_RECOVERY_FAILED"))
}

fn stage_artifacts(
    root: &Path,
    artifacts: &[(&'static str, Vec<u8>)],
) -> Result<Vec<PathBuf>, i32> {
    let nonce = transaction_nonce();
    let mut staged = Vec::with_capacity(artifacts.len());
    for (index, (_, bytes)) in artifacts.iter().enumerate() {
        let path = root.join(format!("{STAGE_PREFIX}{}.{}.{}.tmp", process::id(), nonce, index));
        if let Err(code) = write_new_synced(&path, bytes) {
            cleanup(&staged);
            return Err(code);
        }
        staged.push(path);
    }
    Ok(staged)
}

fn replace_artifacts(
    root: &Path,
    scaffold: &[(&'static str, Vec<u8>)],
    artifacts: &[(&'static str, Vec<u8>)],
    staged: &[PathBuf],
) -> Result<(), i32> {
    for (index, ((name, _), staged_path)) in artifacts.iter().zip(staged).enumerate() {
        let target = root.join(name);
        let expected_scaffold = &scaffold[index].1;
        if !fs::read(&target).is_ok_and(|actual| actual == *expected_scaffold) {
            restore_transaction_owned(root, &scaffold[..index], &artifacts[..index])
                .map_err(|_| fail("AUTOMATION_CAPTURE_ROLLBACK_FAILED"))?;
            return Err(fail("AUTOMATION_CAPTURE_CONCURRENT_MODIFICATION"));
        }
        let bytes = fs::read(staged_path).map_err(|_| fail("AUTOMATION_CAPTURE_STAGE_FAILED"))?;
        if overwrite_synced(&target, &bytes).is_err() {
            restore_transaction_owned(root, &scaffold[..index], &artifacts[..index])
                .map_err(|_| fail("AUTOMATION_CAPTURE_ROLLBACK_FAILED"))?;
            return Err(fail("AUTOMATION_CAPTURE_WRITE_FAILED"));
        }
    }
    Ok(())
}

fn create_journal(path: &Path, journal: &TransactionJournal) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(journal).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("journal path has no parent"))?;
    let temp = parent.join(format!(
        "{STAGE_PREFIX}journal.{}.{}.tmp",
        process::id(),
        transaction_nonce()
    ));
    let result = (|| {
        write_new_synced_io(&temp, &bytes)?;
        fs::hard_link(&temp, path)?;
        Ok(())
    })();
    let _ = fs::remove_file(temp);
    result
}

fn artifacts_match(root: &Path, expected: &[(&'static str, Vec<u8>)]) -> bool {
    expected
        .iter()
        .all(|(name, bytes)| fs::read(root.join(name)).is_ok_and(|actual| actual == *bytes))
}

fn restore_transaction_owned(
    root: &Path,
    scaffold: &[(&'static str, Vec<u8>)],
    artifacts: &[(&'static str, Vec<u8>)],
) -> std::io::Result<()> {
    if scaffold.len() != artifacts.len() {
        return Err(std::io::Error::other("rollback artifact set mismatch"));
    }

    for ((scaffold_name, scaffold_bytes), (artifact_name, artifact_bytes)) in
        scaffold.iter().zip(artifacts)
    {
        if scaffold_name != artifact_name {
            return Err(std::io::Error::other("rollback artifact name mismatch"));
        }
        let path = root.join(scaffold_name);
        let actual = fs::read(&path)?;
        if actual == *scaffold_bytes {
            continue;
        }
        if actual != *artifact_bytes {
            return Err(std::io::Error::other(
                "rollback target changed outside this transaction",
            ));
        }
        overwrite_synced(&path, scaffold_bytes)?;
    }
    Ok(())
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), i32> {
    if write_new_synced_io(path, bytes).is_err() {
        let _ = fs::remove_file(path);
        return Err(fail("AUTOMATION_CAPTURE_STAGE_FAILED"));
    }
    Ok(())
}

fn write_new_synced_io(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
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

fn cleanup(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn cleanup_staged(root: &Path) {
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

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "vpr-automation-capture-{name}-{}-{}",
                process::id(),
                transaction_nonce()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixtures() -> (
        Vec<(&'static str, Vec<u8>)>,
        Vec<(&'static str, Vec<u8>)>,
        TransactionJournal,
    ) {
        let scaffold = vec![
            ("ci-evidence.json", b"ci failed\n".to_vec()),
            ("e2e-evidence.json", b"e2e failed\n".to_vec()),
        ];
        let artifacts = vec![
            ("ci-evidence.json", b"ci passed\n".to_vec()),
            ("e2e-evidence.json", b"e2e passed\n".to_vec()),
        ];
        let journal = TransactionJournal::new(&"a".repeat(40), b"input", &artifacts).unwrap();
        (scaffold, artifacts, journal)
    }

    #[test]
    fn commit_replaces_only_complete_scaffold_and_is_idempotent() {
        let dir = TempDir::new("commit");
        let (scaffold, artifacts, _) = fixtures();
        for (name, bytes) in &scaffold {
            fs::write(dir.0.join(name), bytes).unwrap();
        }
        let transaction =
            AutomationCaptureTransaction::new(&dir.0, &"a".repeat(40), b"input", &scaffold, &artifacts);
        assert_eq!(transaction.commit(), Ok(CaptureOutcome::Committed));
        assert_eq!(transaction.commit(), Ok(CaptureOutcome::AlreadyCommitted));
    }

    #[test]
    fn modified_automation_evidence_is_never_overwritten() {
        let dir = TempDir::new("modified");
        let (scaffold, artifacts, _) = fixtures();
        fs::write(dir.0.join(scaffold[0].0), b"operator edited\n").unwrap();
        fs::write(dir.0.join(scaffold[1].0), &scaffold[1].1).unwrap();
        let transaction =
            AutomationCaptureTransaction::new(&dir.0, &"a".repeat(40), b"input", &scaffold, &artifacts);
        assert_eq!(transaction.commit(), Err(2));
        assert_eq!(
            fs::read(dir.0.join("ci-evidence.json")).unwrap(),
            b"operator edited\n"
        );
    }

    #[test]
    fn recovery_never_overwrites_unrecognized_operator_content() {
        let dir = TempDir::new("ambiguous-recovery");
        let (scaffold, artifacts, journal) = fixtures();
        for (name, bytes) in &scaffold {
            fs::write(dir.0.join(name), bytes).unwrap();
        }
        fs::write(dir.0.join(artifacts[0].0), b"operator-owned\n").unwrap();
        create_journal(&dir.0.join(COMMIT_MARKER), &journal).unwrap();

        let transaction = AutomationCaptureTransaction::new(
            &dir.0,
            &"a".repeat(40),
            b"input",
            &scaffold,
            &artifacts,
        );
        assert_eq!(transaction.commit(), Err(2));
        assert_eq!(
            fs::read(dir.0.join(artifacts[0].0)).unwrap(),
            b"operator-owned\n"
        );
        assert!(dir.0.join(COMMIT_MARKER).exists());
    }

    #[test]
    fn rollback_refuses_to_overwrite_external_change() {
        let dir = TempDir::new("rollback-external-change");
        let (scaffold, artifacts, _) = fixtures();
        for (name, bytes) in &scaffold {
            fs::write(dir.0.join(name), bytes).unwrap();
        }

        fs::write(dir.0.join(artifacts[0].0), &artifacts[0].1).unwrap();
        fs::write(dir.0.join(artifacts[0].0), b"operator changed after replace\n").unwrap();

        assert!(restore_transaction_owned(&dir.0, &scaffold[..1], &artifacts[..1]).is_err());
        assert_eq!(
            fs::read(dir.0.join(artifacts[0].0)).unwrap(),
            b"operator changed after replace\n"
        );
    }

    #[test]
    fn interrupted_matching_capture_rolls_back_before_retry() {
        let dir = TempDir::new("recover");
        let (scaffold, artifacts, journal) = fixtures();
        for (name, bytes) in &scaffold {
            fs::write(dir.0.join(name), bytes).unwrap();
        }
        fs::write(dir.0.join(artifacts[0].0), &artifacts[0].1).unwrap();
        create_journal(&dir.0.join(COMMIT_MARKER), &journal).unwrap();

        let transaction =
            AutomationCaptureTransaction::new(&dir.0, &"a".repeat(40), b"input", &scaffold, &artifacts);
        assert_eq!(transaction.commit(), Ok(CaptureOutcome::Committed));
        assert!(artifacts_match(&dir.0, &artifacts));
        assert!(!dir.0.join(COMMIT_MARKER).exists());
    }
}
