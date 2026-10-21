//! Per-process ownership of an explicitly configured reviewed Persona store.
//! OS-backed advisory locks survive threads, release on crash, and protect
//! the existing canonical storage path without creating a second authority.

use std::env;
use std::fs;
use std::path::Path;

/// Hold this guard for the entire backend lifetime, before loading the Persona.
pub struct PersonaStoreLease {
    _file: Option<fs::File>,
}

/// Refuse a competing process using the same configured reviewed-Persona file.
///
/// # Errors
/// Reports that the store is unavailable, rather than permitting a lost update.
pub fn acquire_persona_store_lease() -> Result<PersonaStoreLease, String> {
    let Some(store_path) = env::var_os("VPR_OWNER_LAB_PERSONA_STORE_PATH")
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
    else {
        // The existing Windows Credential Manager path remains unchanged.
        // On non-Windows platforms saving without an explicit path fails closed.
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
