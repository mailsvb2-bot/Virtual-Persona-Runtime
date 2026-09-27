use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use vpr_evaluation::{RT0_MANUAL_SUPPORTING_FILES, sha256_hex};

use super::fail;

const LOCK_FILE: &str = ".rt0-supporting-capture.lock";
const COMMIT_MARKER: &str = ".rt0-supporting-capture.commit";
const STAGE_PREFIX: &str = ".rt0-supporting-capture.stage.";
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
    file: File,
}

impl CaptureLock {
    pub(super) fn acquire(
        root: &Path,
        journal: &TransactionJournal,
        expected: &[(&'static str, String)],
        artifacts: &[(&'static str, Vec<u8>)],
    ) -> Result<LockOutcome, i32> {
        let path = root.join(LOCK_FILE);
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|_| fail("CAPTURE_LOCK_FAILED"))?;

        if !platform_lock::try_lock_exclusive(&file).map_err(|_| fail("CAPTURE_LOCK_FAILED"))? {
            return Err(fail("CAPTURE_IN_PROGRESS"));
        }

        let previous = read_lock_journal(&mut file)?;
        let lock = Self { file };
        recover_under_lock(root, journal, previous.as_ref(), expected, artifacts)?;
        write_lock_journal(&lock.file, journal)?;

        if artifacts_match(root, artifacts) {
            Ok(LockOutcome::AlreadyCommitted)
        } else {
            Ok(LockOutcome::Acquired(lock))
        }
    }
}

impl Drop for CaptureLock {
    fn drop(&mut self) {
        let _ = platform_lock::unlock(&self.file);
    }
}

fn recover_under_lock(
    root: &Path,
    journal: &TransactionJournal,
    previous: Option<&TransactionJournal>,
    expected: &[(&'static str, String)],
    artifacts: &[(&'static str, Vec<u8>)],
) -> Result<(), i32> {
    let marker = root.join(COMMIT_MARKER);
    if marker.exists() {
        let marker_journal = read_journal(&marker)?;
        if marker_journal != *journal {
            return Err(fail("STALE_CAPTURE_MISMATCH"));
        }
        if !restore_scaffold(root, expected) {
            return Err(fail("STALE_CAPTURE_RECOVERY_FAILED"));
        }
        cleanup_staged_files(root);
        fs::remove_file(marker).map_err(|_| fail("STALE_CAPTURE_RECOVERY_FAILED"))?;
        return Ok(());
    }

    if artifacts_match(root, artifacts) || scaffold_matches(root, expected) {
        cleanup_staged_files(root);
        return Ok(());
    }

    if previous.is_some_and(|previous| previous != journal) {
        return Err(fail("STALE_CAPTURE_MISMATCH"));
    }
    Err(fail("STALE_CAPTURE_AMBIGUOUS"))
}

fn read_lock_journal(file: &mut File) -> Result<Option<TransactionJournal>, i32> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| fail("CAPTURE_LOCK_FAILED"))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| fail("CAPTURE_LOCK_FAILED"))?;
    if bytes.is_empty() {
        return Ok(None);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| fail("STALE_CAPTURE_JOURNAL_INVALID"))
}

fn write_lock_journal(file: &File, journal: &TransactionJournal) -> Result<(), i32> {
    let mut file = file;
    let mut bytes = serde_json::to_vec(journal).map_err(|_| fail("CAPTURE_LOCK_FAILED"))?;
    bytes.push(b'\n');
    file.set_len(0).map_err(|_| fail("CAPTURE_LOCK_FAILED"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| fail("CAPTURE_LOCK_FAILED"))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| fail("CAPTURE_LOCK_FAILED"))
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

#[cfg(unix)]
mod platform_lock {
    use std::fs::File;
    use std::io;
    use std::os::fd::AsRawFd;

    pub(super) fn try_lock_exclusive(file: &File) -> io::Result<bool> {
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EWOULDBLOCK) => Ok(false),
            _ => Err(error),
        }
    }

    pub(super) fn unlock(file: &File) -> io::Result<()> {
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(windows)]
mod platform_lock {
    use std::fs::File;
    use std::io;
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, HANDLE};
    use windows_sys::Win32::Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx, UnlockFileEx,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;

    pub(super) fn try_lock_exclusive(file: &File) -> io::Result<bool> {
        let mut overlapped = unsafe { std::mem::zeroed::<OVERLAPPED>() };
        let result = unsafe {
            LockFileEx(
                file.as_raw_handle() as HANDLE,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                u32::MAX,
                u32::MAX,
                &mut overlapped,
            )
        };
        if result != 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
            Ok(false)
        } else {
            Err(error)
        }
    }

    pub(super) fn unlock(file: &File) -> io::Result<()> {
        let mut overlapped = unsafe { std::mem::zeroed::<OVERLAPPED>() };
        let result = unsafe {
            UnlockFileEx(
                file.as_raw_handle() as HANDLE,
                0,
                u32::MAX,
                u32::MAX,
                &mut overlapped,
            )
        };
        if result != 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
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
    fn live_lock_cannot_expire_or_be_stolen() {
        let root = temp_dir("lock");
        let Fixtures {
            expected,
            artifacts,
            journal,
        } = fixtures();
        for (name, content) in &expected {
            fs::write(root.join(name), content).unwrap();
        }

        let first = match CaptureLock::acquire(&root, &journal, &expected, &artifacts).unwrap() {
            LockOutcome::Acquired(lock) => lock,
            LockOutcome::AlreadyCommitted => panic!("unexpected committed recovery"),
        };
        assert_eq!(
            CaptureLock::acquire(&root, &journal, &expected, &artifacts).err(),
            Some(2)
        );
        drop(first);
        assert!(CaptureLock::acquire(&root, &journal, &expected, &artifacts).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn orphan_commit_marker_is_recovered_after_process_lock_release() {
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

        let recovered = CaptureLock::acquire(&root, &journal, &expected, &artifacts).unwrap();
        assert!(matches!(recovered, LockOutcome::Acquired(_)));
        assert!(!root.join(COMMIT_MARKER).exists());
        assert!(scaffold_matches(&root, &expected));
        drop(recovered);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn completed_capture_is_recognized_after_process_lock_release() {
        let root = temp_dir("complete");
        let Fixtures {
            expected,
            artifacts,
            journal,
        } = fixtures();
        for (name, bytes) in &artifacts {
            fs::write(root.join(name), bytes).unwrap();
        }

        let recovered = CaptureLock::acquire(&root, &journal, &expected, &artifacts).unwrap();
        assert!(matches!(recovered, LockOutcome::AlreadyCommitted));
        assert!(artifacts_match(&root, &artifacts));
        drop(recovered);
        let _ = fs::remove_dir_all(root);
    }
}
