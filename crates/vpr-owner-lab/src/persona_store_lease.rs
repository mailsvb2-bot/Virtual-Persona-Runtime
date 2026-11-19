//! Per-process ownership of an explicitly configured reviewed Persona store.
//! OS-backed advisory locks survive threads, release on crash, and protect
//! the existing canonical storage path without creating a second authority.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

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
        return Ok(Some(PathBuf::from(path)));
    }
    #[cfg(windows)]
    {
        let root = env::var_os("LOCALAPPDATA")
            .or_else(|| env::var_os("APPDATA"))
            .ok_or_else(|| "Windows Persona lock directory unavailable".to_owned())?;
        return Ok(Some(
            PathBuf::from(root)
                .join("Virtual-Persona-Runtime")
                .join("owner-lab-credential-manager-v2"),
        ));
    }
    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

/// Hold this guard for the entire backend lifetime, before loading the Persona.
pub struct PersonaStoreLease {
    _file: Option<fs::File>,
}

/// Refuse a competing process using the same configured reviewed-Persona file.
///
/// # Errors
/// Reports that the store is unavailable, rather than permitting a lost update.
pub fn acquire_persona_store_lease() -> Result<PersonaStoreLease, String> {
    let Some(store_path) = lock_target()? else {
        // Only the unconfigured non-Windows Owner Lab is ephemeral.
        return Ok(PersonaStoreLease { _file: None });
    };
    let parent = store_path
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|_| "reviewed Persona lease directory creation failed")?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|_| "reviewed Persona lease directory resolution failed")?;
    let name = store_path
        .file_name()
        .ok_or_else(|| "reviewed Persona store path must name a file".to_owned())?;
    let mut lock_name = name.to_os_string();
    lock_name.push(".owner.lock");
    let lock_path = canonical_parent.join(lock_name);
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
    Ok(PersonaStoreLease { _file: Some(file) })
}
