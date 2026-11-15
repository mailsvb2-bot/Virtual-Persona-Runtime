//! Single RT1 visitor START error mapper: never mistake unresolved remote
//! CREATE/STOP for an ordinary admission denial or expose provider internals.
use super::{HttpResponse, error_response};

pub(super) fn startup_error_response(reason: &str) -> HttpResponse {
    match reason {
        "RT1_REMOTE_CREATE_UNCONFIRMED" => {
            // The provider may have created media without returning an ID.
            // Keep the already-registered STOP reconciler as the sole owner.
            error_response(502, "RT1_REMOTE_CREATE_UNCONFIRMED")
        }
        "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED"
        | "RT1_PROVIDER_START_FAILED_STOP_UNCONFIRMED"
        | "RT1_PROVIDER_START_REVOKED_STOP_UNCONFIRMED" => {
            error_response(502, "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED")
        }
        _ => error_response(409, "RT1_VISITOR_START_DENIED"),
    }
}

/// Report a specific safe readiness blocker without exposing provider
/// credentials, owner identity or other private publication failures.
pub(super) fn publication_error_response(reason: &str) -> HttpResponse {
    match reason {
        "RT1_REMOTE_VIDEO_STOP_UNCONFIRMED" => {
            error_response(503, "RT1_REMOTE_VIDEO_STOP_UNCONFIRMED")
        }
        "RT1_WORKER_NOT_CONFIGURED" => error_response(503, "RT1_WORKER_NOT_CONFIGURED"),
        "RT1_PROVIDER_UNAVAILABLE" => error_response(503, "RT1_PROVIDER_UNAVAILABLE"),
        "RT1_TTS_NOT_CONFIGURED" => error_response(503, "RT1_TTS_NOT_CONFIGURED"),
        _ => error_response(409, "RT1_PUBLISH_DENIED"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_reports_verifiable_media_transport_blockers() {
        for code in [
            "RT1_REMOTE_VIDEO_STOP_UNCONFIRMED",
            "RT1_WORKER_NOT_CONFIGURED",
            "RT1_PROVIDER_UNAVAILABLE",
            "RT1_TTS_NOT_CONFIGURED",
        ] {
            assert_eq!(publication_error_response(code).status_code().0, 503);
        }
        for code in [
            "STALE_PERSONA_VERSION",
            "invalid owner token",
            "unexpected provider detail",
        ] {
            assert_eq!(publication_error_response(code).status_code().0, 409);
        }
    }

    #[test]
    fn unresolved_remote_create_requires_reconciliation_not_client_retry() {
        assert_eq!(
            startup_error_response("RT1_REMOTE_CREATE_UNCONFIRMED")
                .status_code()
                .0,
            502
        );
    }

    #[test]
    fn unconfirmed_stops_never_become_ordinary_409_denials() {
        for reason in [
            "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED",
            "RT1_PROVIDER_START_FAILED_STOP_UNCONFIRMED",
            "RT1_PROVIDER_START_REVOKED_STOP_UNCONFIRMED",
        ] {
            assert_eq!(startup_error_response(reason).status_code().0, 502);
        }
    }

    #[test]
    fn all_other_denials_remain_generic_and_redacted() {
        for reason in [
            "CONSENT_REQUIRED",
            "RT1_VISITOR_REVOKED_DURING_CREATE",
            "internal provider secrets must never escape",
        ] {
            assert_eq!(startup_error_response(reason).status_code().0, 409);
        }
    }
}
