//! Fail-closed RT1 publication lease recovery at Owner Lab process startup.
//! The same canonical Persona store and its existing exclusive lease are used.
use std::env;
use std::ffi::OsStr;

/// Invalidate persisted visitor session leases before binding the HTTP listener.
/// Ephemeral non-Windows labs have no durable publication to recover.
pub(super) fn recover_before_listener() -> Result<(), String> {
    if recovery_required(
        cfg!(windows),
        env::var_os("VPR_OWNER_LAB_PERSONA_STORE_PATH").as_deref(),
    ) {
        vpr_owner_lab::Rt1WorkspaceService::new()
            .recover_before_listener()
            .map_err(|_| "RT1 publication recovery failed before listener startup".to_owned())?;
    }
    Ok(())
}

/// Match persona_persistence::explicit_store_path: empty means unconfigured.
fn recovery_required(on_windows: bool, store_path: Option<&OsStr>) -> bool {
    on_windows || store_path.is_some_and(|path| !path.is_empty())
}

#[cfg(test)]
mod tests {
    use super::recovery_required;
    use std::ffi::OsStr;

    #[test]
    fn ephemeral_non_windows_lab_does_not_require_durable_recovery() {
        assert!(!recovery_required(false, None));
        assert!(!recovery_required(false, Some(OsStr::new(""))));
    }

    #[test]
    fn configured_or_windows_store_must_recover_before_listener() {
        assert!(recovery_required(false, Some(OsStr::new("persona.json"))));
        assert!(recovery_required(true, None));
        assert!(recovery_required(true, Some(OsStr::new(""))));
    }
}
