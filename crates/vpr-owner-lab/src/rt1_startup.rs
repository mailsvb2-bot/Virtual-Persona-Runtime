//! Fail-closed RT1 publication lease recovery at Owner Lab process startup.
//! The same canonical Persona store and its existing exclusive lease are used.
use std::env;
use std::ffi::OsStr;
use std::time::Duration;

use vpr_provider_local_open_source::operation_client::RemoteOperationClient;

/// Invalidate persisted visitor session leases before binding the HTTP listener.
/// Ephemeral non-Windows labs have no durable publication to recover.
pub(super) fn recover_before_listener() -> Result<(), String> {
    let publication = if recovery_required(
        cfg!(windows),
        env::var_os("VPR_OWNER_LAB_PERSONA_STORE_PATH").as_deref(),
    ) {
        vpr_owner_lab::Rt1WorkspaceService::new()
            .recover_before_listener()
            .map_err(|_| "RT1 publication recovery failed before listener startup".to_owned())
    } else {
        Ok(())
    };
    // Even a corrupt/unavailable Persona store must not skip best-effort
    // remote sender cleanup. Startup still fails if either authority recovery
    // or the worker's verified audio/video STOP remains unresolved.
    let worker = recover_worker_before_listener();
    publication?;
    worker
}

/// Reconcile the worker's durable operation inventory after Owner Lab loses
/// its in-memory visitor registry. The worker remains the canonical STOP owner.
fn recover_worker_before_listener() -> Result<(), String> {
    let selected = env::var("VPR_RT1_CANONICAL_VOICE_WORKER").as_deref() == Ok("true");
    let endpoint = env::var_os("RT1_WORKER_ENDPOINT");
    let token = env::var_os("RT1_WORKER_TOKEN");
    let required = worker_recovery_required(selected, endpoint.is_some(), token.is_some())
        .map_err(str::to_owned)?;
    if !required {
        return Ok(());
    }
    let endpoint = endpoint
        .and_then(|value| value.into_string().ok())
        .ok_or_else(|| "RT1 worker recovery endpoint is invalid".to_owned())?;
    let token = token
        .and_then(|value| value.into_string().ok())
        .ok_or_else(|| "RT1 worker recovery token is invalid".to_owned())?;
    let client = RemoteOperationClient::new(&endpoint, &token, Duration::from_secs(5))
        .map_err(|_| "RT1 worker recovery client unavailable".to_owned())?;
    client.revoke_unresolved().map_err(|_| {
        "RT1 worker audio/video STOP recovery is unconfirmed; listener startup denied".to_owned()
    })?;
    Ok(())
}

fn worker_recovery_required(
    selected: bool,
    endpoint_present: bool,
    token_present: bool,
) -> Result<bool, &'static str> {
    // Diagnostic and D-ID fixture routes may carry worker credentials in their
    // environment but do not own a canonical worker operation.
    if !selected {
        return Ok(false);
    }
    match (endpoint_present, token_present) {
        (true, true) => Ok(true),
        _ => Err("RT1 worker recovery credentials are incomplete"),
    }
}

/// Match `persona_persistence::explicit_store_path`: empty means unconfigured.
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

    #[test]
    fn worker_recovery_runs_only_for_selected_canonical_worker_and_requires_credentials() {
        use super::worker_recovery_required;

        assert_eq!(worker_recovery_required(false, false, false), Ok(false));
        assert_eq!(worker_recovery_required(false, true, true), Ok(false));
        assert_eq!(worker_recovery_required(true, true, true), Ok(true));
        assert!(worker_recovery_required(true, false, false).is_err());
        assert!(worker_recovery_required(true, true, false).is_err());
        assert!(worker_recovery_required(true, false, true).is_err());
    }
}
