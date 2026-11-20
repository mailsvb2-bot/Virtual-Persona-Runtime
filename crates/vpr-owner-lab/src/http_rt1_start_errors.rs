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
        "RT1_WORKER_NOT_READY" => error_response(503, "RT1_WORKER_NOT_READY"),
        "RT1_VIDEO_RENDERER_UNAVAILABLE" => error_response(503, "RT1_VIDEO_RENDERER_UNAVAILABLE"),
        _ => error_response(409, "RT1_VISITOR_START_DENIED"),
    }
}

/// Report a specific safe readiness blocker without exposing provider
/// credentials, owner identity or other private publication failures.
pub(super) fn publication_error_response(reason: &str) -> HttpResponse {
    match reason {
        "RT1_PUBLIC_RELEASE_QUALIFICATION_REQUIRED" => {
            error_response(503, "RT1_PUBLIC_RELEASE_QUALIFICATION_REQUIRED")
        }
        "RT1_PUBLIC_ORIGIN_INVALID" => error_response(503, "RT1_PUBLIC_ORIGIN_INVALID"),
        "RT1_REMOTE_VIDEO_STOP_UNCONFIRMED" => {
            error_response(503, "RT1_REMOTE_VIDEO_STOP_UNCONFIRMED")
        }
        "RT1_WORKER_NOT_CONFIGURED" => error_response(503, "RT1_WORKER_NOT_CONFIGURED"),
        "RT1_WORKER_NOT_READY" => error_response(503, "RT1_WORKER_NOT_READY"),
        "RT1_VIDEO_RENDERER_UNAVAILABLE" => error_response(503, "RT1_VIDEO_RENDERER_UNAVAILABLE"),
        "RT1_PROVIDER_UNAVAILABLE" => error_response(503, "RT1_PROVIDER_UNAVAILABLE"),
        "RT1_TTS_NOT_CONFIGURED" => error_response(503, "RT1_TTS_NOT_CONFIGURED"),
        _ => error_response(409, "RT1_PUBLISH_DENIED"),
    }
}

/// Preserve only reviewed stable RT0 reason codes for a visitor turn.
/// Unknown errors (including provider messages/credentials) are redacted.
/// This is a response mapper, never a second policy or provider decision.
pub(super) fn turn_error_response(reason: &str, voice: bool) -> HttpResponse {
    let (status, code) = match reason {
        "RT1_VISITOR_REVOKED" | "AUTH_REVOKED" => (409, "AUTH_REVOKED"),
        "AUTH_EXPIRED" => (409, "AUTH_EXPIRED"),
        "AUTH_SCOPE_DENIED" => (403, "AUTH_SCOPE_DENIED"),
        "CONSENT_REQUIRED" => (403, "CONSENT_REQUIRED"),
        "EGRESS_DENIED" | "EGRESS_LOCAL_ONLY" => (403, "EGRESS_DENIED"),
        "PROVIDER_RATE_LIMITED" => (429, "PROVIDER_RATE_LIMITED"),
        "PROVIDER_TIMEOUT" => (504, "PROVIDER_TIMEOUT"),
        "PROVIDER_UNAVAILABLE" | "PROVIDER_AUTH_DENIED" => (503, "PROVIDER_UNAVAILABLE"),
        "BUDGET_EXHAUSTED" => (409, "BUDGET_EXHAUSTED"),
        "TURN_CANCELLED" => (409, "TURN_CANCELLED"),
        "PREPARATION_FAILED" => (503, "PREPARATION_FAILED"),
        "INVALID_INPUT" => (400, "INVALID_INPUT"),
        "RT1_MICROPHONE_QUEUE_UNAVAILABLE" if voice => (503, "RT1_MICROPHONE_QUEUE_UNAVAILABLE"),
        "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED" => (502, "RT1_REMOTE_MEDIA_STOP_UNCONFIRMED"),
        _ if voice => (409, "RT1_VISITOR_VOICE_DENIED"),
        _ => (409, "RT1_VISITOR_TEXT_DENIED"),
    };
    error_response(status, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visitor_turn_reasons_keep_only_safe_canonical_codes() {
        for (reason, status) in [
            ("AUTH_REVOKED", 409),
            ("AUTH_SCOPE_DENIED", 403),
            ("PROVIDER_UNAVAILABLE", 503),
            ("PROVIDER_RATE_LIMITED", 429),
            ("PROVIDER_TIMEOUT", 504),
            ("BUDGET_EXHAUSTED", 409),
            ("TURN_CANCELLED", 409),
            ("RT1_MICROPHONE_QUEUE_UNAVAILABLE", 503),
        ] {
            assert_eq!(turn_error_response(reason, true).status_code().0, status);
        }
        // Unknown provider errors may contain secrets: never project them.
        assert_eq!(
            turn_error_response("api-key=private", true).status_code().0,
            409
        );
        assert_eq!(
            turn_error_response("api-key=private", false)
                .status_code()
                .0,
            409
        );
        assert_eq!(
            turn_error_response("RT1_MICROPHONE_QUEUE_UNAVAILABLE", false)
                .status_code()
                .0,
            409
        );
    }

    #[test]
    fn publication_reports_verifiable_media_transport_blockers() {
        for code in [
            "RT1_REMOTE_VIDEO_STOP_UNCONFIRMED",
            "RT1_WORKER_NOT_CONFIGURED",
            "RT1_WORKER_NOT_READY",
            "RT1_VIDEO_RENDERER_UNAVAILABLE",
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
    fn stale_visitor_tokens_are_denied_when_worker_or_renderer_is_not_ready() {
        for code in ["RT1_WORKER_NOT_READY", "RT1_VIDEO_RENDERER_UNAVAILABLE"] {
            assert_eq!(startup_error_response(code).status_code().0, 503);
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
