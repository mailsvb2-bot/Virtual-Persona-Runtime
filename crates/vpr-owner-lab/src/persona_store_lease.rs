//! Per-process ownership of an explicitly configured reviewed Persona store.
//! OS-backed advisory locks survive threads, release on crash, and protect
//! the existing canonical storage path without creating a second authority.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};

use parking_lot::Mutex;

/// One stable lock identity for BOTH the lifetime service lease and every
/// short canonical read-modify-commit. The implicit Windows Credential Manager
/// record has no file path, so use a private per-user sibling-lock namespace.
///
/// # Errors
/// Fails closed when the implicit Windows per-user lock directory is unavailable.
pub(crate) fn lock_target() -> Result<Option<PathBuf>, String> {
    if let Some(path) =
        env::var_os("VPR_OWNER_LAB_PERSONA_STORE_PATH").filter(|value| !value.is_empty())
    {
        let path = PathBuf::from(path);
        if path.file_name().is_none() {
            return Err("reviewed Persona store path must name a file".into());
        }
        return Ok(Some(path));
    }
    #[cfg(windows)]
    {
        let root = env::var_os("LOCALAPPDATA")
            .or_else(|| env::var_os("APPDATA"))
            .ok_or_else(|| "Windows Persona lock directory unavailable".to_owned())?;
        Ok(Some(
            PathBuf::from(root)
                .join("Virtual-Persona-Runtime")
                .join("owner-lab-credential-manager-v2"),
        ))
    }
    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

// An in-process mirror of the existing exclusive OS lease, not a separate
// authority. Weak references expire on Drop/crash and cannot prolong ownership.
struct HeldLease {
    _file: fs::File,
    path: PathBuf,
}
type ActiveLease = Mutex<Option<Weak<HeldLease>>>;
static ACTIVE_LEASE: OnceLock<ActiveLease> = OnceLock::new();

/// Hold this guard for the entire backend lifetime, before loading the Persona.
pub struct PersonaStoreLease {
    _owner: Option<Arc<HeldLease>>,
}

/// Live media admission is legal only in the exact process holding the existing
/// exclusive Owner Lab lease. A process with direct access to the Persona file
/// must not start or continue a separate media/session registry.
///
/// # Errors
/// Denies an unconfigured, dropped, or independently owned OS lease.
pub(crate) fn require_live_runtime_owner() -> Result<(), String> {
    let path = owner_lock_path()?.ok_or("RT1 owner lease required")?;
    let guard = ACTIVE_LEASE.get().ok_or("RT1 owner lease required")?.lock();
    let owner = guard.as_ref().and_then(Weak::upgrade)
        .ok_or("RT1 owner lease required")?;
    if owner.path != path {
        return Err("RT1 owner lease does not match canonical Persona".into());
    }
    Ok(())
}

fn owner_lock_path() -> Result<Option<PathBuf>, String> {
    let Some(store_path) = lock_target()? else {
        return Ok(None);
    };
    let parent = store_path
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|_| "reviewed Persona lease directory resolution failed")?;
    let name = store_path.file_name()
        .ok_or_else(|| "reviewed Persona store path must name a file".to_owned())?;
    let mut lock_name = name.to_os_string();
    lock_name.push(".owner.lock");
    Ok(Some(canonical_parent.join(lock_name)))
}

/// Refuse a competing process using the same configured reviewed-Persona file.
///
/// # Errors
/// Reports that the store is unavailable, rather than permitting a lost update.
pub fn acquire_persona_store_lease() -> Result<PersonaStoreLease, String> {
    let Some(store_path) = lock_target()? else {
        // Unconfigured non-Windows labs cannot admit live RT1 visitors.
        return Ok(PersonaStoreLease { _owner: None });
    };
    let parent = store_path
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|_| "reviewed Persona lease directory creation failed")?;
    let lock_path = owner_lock_path()?.ok_or("reviewed Persona lease path unavailable")?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&lock_path)
        .map_err(|_| "reviewed Persona lease file cannot be opened")?;
    fs2::FileExt::try_lock_exclusive(&file)
        .map_err(|_| "reviewed Persona store is already in use by another process")?;
    let owner = Arc::new(HeldLease { _file: file, path: lock_path });
    *ACTIVE_LEASE.get_or_init(|| Mutex::new(None)).lock() = Some(Arc::downgrade(&owner));
    Ok(PersonaStoreLease { _owner: Some(owner) })
}

#[cfg(test)]
#[path = "persona_store_lease_tests.rs"]
mod tests;
