//! One durable transaction fence for the existing canonical Persona file.
//! Never lock the atomically replaced data inode: use a stable sibling lock.
//! The process-local mutex and the OS file lock share the same lock order.
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, MutexGuard};

pub(crate) static CANONICAL_TRANSACTION: Mutex<()> = Mutex::new(());
const MAX_LOCK_WAIT: Duration = Duration::from_secs(2);

pub(crate) struct PersonaTransaction {
    _local: MutexGuard<'static, ()>,
    _file: Option<File>,
}

/// A short, read-modify-commit transaction on the currently selected Persona.
/// The caller must NOT perform network/provider I/O while holding this guard.
pub(crate) fn begin() -> Result<PersonaTransaction, String> {
    let path = std::env::var_os("VPR_OWNER_LAB_PERSONA_STORE_PATH")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    begin_optional(path.as_deref())
}

/// Used by explicit-path persistence tests and RT0 reviewed-capture writes;
/// the lock follows the destination, not process-global test environment.
pub(crate) fn begin_for_path(path: &Path) -> Result<PersonaTransaction, String> {
    begin_optional(Some(path))
}

fn begin_optional(path: Option<&Path>) -> Result<PersonaTransaction, String> {
    let local = CANONICAL_TRANSACTION.lock();
    let file = match path {
        Some(path) => Some(acquire_sibling_lock(path)?),
        None => None, // Existing Windows Credential Manager is single-process only.
    };
    Ok(PersonaTransaction {
        _local: local,
        _file: file,
    })
}

fn acquire_sibling_lock(path: &Path) -> Result<File, String> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|_| "Persona transaction directory unavailable")?;
    let parent = fs::canonicalize(parent)
        .map_err(|_| "Persona transaction directory unavailable")?;
    let filename = path.file_name()
        .ok_or_else(|| "Persona transaction requires a file path".to_owned())?;
    let mut lock_name = filename.to_os_string();
    lock_name.push(".transaction.lock");
    let lock_path = parent.join(lock_name);
    // Never follow a pre-existing symlink to another filesystem object.
    match fs::symlink_metadata(&lock_path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
            return Err("Persona transaction lock path invalid".into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("Persona transaction lock inaccessible".into()),
        _ => {}
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&lock_path)
        .map_err(|_| "Persona transaction lock inaccessible")?;
    let opened = file.metadata().map_err(|_| "Persona transaction lock inaccessible")?;
    let current = fs::symlink_metadata(&lock_path)
        .map_err(|_| "Persona transaction lock inaccessible")?;
    if current.file_type().is_symlink() || !current.is_file() {
        return Err("Persona transaction lock path invalid".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (opened.dev(), opened.ino()) != (current.dev(), current.ino()) {
            return Err("Persona transaction lock changed during open".into());
        }
    }
    let deadline = Instant::now() + MAX_LOCK_WAIT;
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
                && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return Err("Persona transaction lock unavailable".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn child_transaction_lock_probe() {
        let Some(path) = std::env::var_os("VPR_TEST_CHILD_PERSONA_LOCK") else {
            return;
        };
        // The parent is holding the lock. A competing process cannot enter.
        assert!(begin_for_path(Path::new(&path)).is_err());
    }

    #[test]
    fn competing_process_cannot_enter_uncommitted_persona_transaction() {
        let path = std::env::temp_dir().join(format!(
            "vpr-multiprocess-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let held = begin_for_path(&path).unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("persona_transaction::tests::child_transaction_lock_probe")
            .arg("--nocapture")
            .env("VPR_TEST_CHILD_PERSONA_LOCK", &path)
            .output().unwrap();
        assert!(output.status.success(),
            "other process entered locked canonical record: {}",
            String::from_utf8_lossy(&output.stderr));
        drop(held);
        // A fresh transaction is available after the first guard is released.
        assert!(begin_for_path(&path).is_ok());
        let mut name = path.as_os_str().to_os_string();
        name.push(".transaction.lock");
        let _ = fs::remove_file(PathBuf::from(name));
    }
}
