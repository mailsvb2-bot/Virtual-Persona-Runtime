use reqwest::blocking::Response;
use vpr_integration::{ProviderError, ProviderErrorKind};

/// D-ID-specific access failure used only for safe credential diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DidRuntimeAccessFailure {
    /// D-ID returned HTTP 401.
    Unauthorized,
    /// D-ID returned HTTP 403 for the runtime access path.
    Forbidden,
    /// A non-authentication provider failure occurred.
    Provider(ProviderError),
}

pub(super) fn validate_audio_url(audio_url: &str) -> Result<(), ProviderError> {
    let parsed = reqwest::Url::parse(audio_url).map_err(|_| invalid_response())?;
    let has_host = parsed
        .host_str()
        .is_some_and(|host| !host.trim().is_empty());
    let has_userinfo = !parsed.username().is_empty() || parsed.password().is_some();
    if parsed.scheme() == "https" && has_host && !has_userinfo {
        Ok(())
    } else {
        Err(policy_denied())
    }
}

pub(super) fn validate_endpoint(endpoint: &str) -> Result<reqwest::Url, ProviderError> {
    let parsed = reqwest::Url::parse(endpoint).map_err(|_| invalid_response())?;
    let loopback = parsed.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    let secure_scheme = parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback);
    let clean_authority = parsed.username().is_empty() && parsed.password().is_none();
    if secure_scheme
        && clean_authority
        && parsed.host_str().is_some()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
    {
        Ok(parsed)
    } else {
        Err(policy_denied())
    }
}

pub(super) fn expect_success(response: Response) -> Result<Response, ProviderError> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(map_status(response.status().as_u16()))
    }
}

fn map_status(status: u16) -> ProviderError {
    match status {
        401 | 403 => policy_denied(),
        408 => ProviderError {
            kind: ProviderErrorKind::Timeout,
            retryable: true,
        },
        429 => ProviderError {
            kind: ProviderErrorKind::RateLimited,
            retryable: true,
        },
        500..=599 => ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: true,
        },
        _ => invalid_response(),
    }
}

pub(super) fn map_transport_error(error: &reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError {
            kind: ProviderErrorKind::Timeout,
            retryable: true,
        }
    } else {
        ProviderError {
            kind: ProviderErrorKind::Unavailable,
            retryable: true,
        }
    }
}

pub(super) const fn cancelled() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Cancelled,
        retryable: false,
    }
}

pub(super) const fn invalid_response() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::InvalidResponse,
        retryable: false,
    }
}

pub(super) const fn unavailable() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::Unavailable,
        retryable: false,
    }
}

pub(super) const fn policy_denied() -> ProviderError {
    ProviderError {
        kind: ProviderErrorKind::PolicyDenied,
        retryable: false,
    }
}
