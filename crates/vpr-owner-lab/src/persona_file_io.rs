//! Atomic, single-commit file IO for the canonical reviewed Persona envelope.
//! No independent publication or owner-state store exists.
use std::fs;
use std::io::Write;
use std::path::Path;

pub(crate) fn read_raw(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(raw) => Ok(Some(raw)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("reviewed Persona file store read failed".into()),
    }
}

pub(crate) fn commit_raw(path: &Path, raw: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|_| "reviewed Persona file store directory creation failed")?;
    // Exclusive, randomized sibling files avoid predictable .tmp collisions and
    // accidental truncation of a different in-flight writer's temporary file.
    // The rename keeps the prior committed file intact until its replacement
    // has been written and flushed. This is not a cross-process CAS store:
    // RT1 must still introduce authoritative revision checks before concurrent use.
    let mut staged = None;
    for _ in 0..16 {
        let mut suffix = [0u8; 16];
        getrandom::fill(&mut suffix)
            .map_err(|_| "reviewed Persona file store temporary name failed")?;
        let mut random = String::with_capacity(32);
        for byte in suffix {
            use std::fmt::Write as _;
            write!(&mut random, "{byte:02x}").expect("writing to String cannot fail");
        }
        let temp = parent.join(format!(".vpr-persona-{random}.tmp"));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&temp) {
            Ok(file) => {
                staged = Some((temp, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err("reviewed Persona file store staging failed".into()),
        }
    }
    let Some((temp, mut file)) = staged else {
        return Err("reviewed Persona file store staging exhausted".into());
    };
    let commit = (|| {
        file.write_all(raw)
            .map_err(|_| "reviewed Persona file store write failed")?;
        file.sync_all()
            .map_err(|_| "reviewed Persona file store sync failed")?;
        drop(file);
        fs::rename(&temp, path).map_err(|_| "reviewed Persona file store commit failed")?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| "reviewed Persona directory sync failed")?;
        Ok::<(), String>(())
    })();
    if commit.is_err() {
        let _ = fs::remove_file(&temp);
    }
    commit
}
