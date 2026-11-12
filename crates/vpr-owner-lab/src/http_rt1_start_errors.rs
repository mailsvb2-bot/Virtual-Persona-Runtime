//! Single RT1 visitor START error mapper: never mistake unresolved remote
//! CREATE/STOP for an ordinary admission denial or expose provider internals.
use super::{HttpResponse, error_response};

pub(super) fn startup_error_response(reason: String) -> HttpResponse {
    match reason.as_str() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_remote_create_requires_reconciliation_not_client_retry() {
        assert_eq!(
            startup_error_response("RT1_REMOTE_CREATE_UNCONFIRMED".into())
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
            assert_eq!(startup_error_response(reason.into()).status_code().0, 502);
        }
    }

    #[test]
    fn all_other_denials_remain_generic_and_redacted() {
        for reason in [
            "CONSENT_REQUIRED",
            "RT1_VISITOR_REVOKED_DURING_CREATE",
            "internal provider secrets must never escape",
        ] {
            assert_eq!(startup_error_response(reason.into()).status_code().0, 409);
        }
    }
}
