//! Pure validation of the server-approved RT1 public visitor HTTPS origin.
//! This module owns no network listener, owner identity or publication policy.

/// Read the server's explicit public DNS origin; absence retains the local lab.
///
/// A malformed configured origin is not silently replaced with localhost.
pub(super) fn configured() -> Result<Option<String>, ()> {
    match std::env::var("VPR_RT1_PUBLIC_VISITOR_ORIGIN") {
        Ok(raw) => verified(&raw).map(Some).ok_or(()),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => Err(()),
    }
}

/// Production publication is forbidden while rights and cost control remain
/// contract-only spikes. This check uses existing canonical contract maturity,
/// not a browser claim or a new policy authority. Contract promotion alone is
/// NOT sufficient to enable publication: a reviewed runtime gate must replace
/// this deny before production. Local RT1 test publication remains unaffected.
pub(super) fn publication_qualification() -> Result<(), &'static str> {
    let origin = configured().map_err(|()| "RT1_PUBLIC_ORIGIN_INVALID")?;
    publication_qualification_for(origin.as_deref())
}

fn publication_qualification_for(origin: Option<&str>) -> Result<(), &'static str> {
    if origin.is_none() {
        return Ok(());
    }
    // No rights/consent revision and budget reservation are currently bound
    // to public session admission. Deny before a shareable token is minted.
    Err("RT1_PUBLIC_RELEASE_QUALIFICATION_REQUIRED")
}

/// Deny every chargeable/public visitor operation when an unqualified
/// HTTPS edge is configured, including tokens minted before configuration.
/// Keep STOP/close reachable to drain an already allocated sender.
pub(super) fn blocked_visitor_route(path: &str) -> Option<&'static str> {
    if !path.starts_with("/api/rt1/visitor/")
        || matches!(path, "/api/rt1/visitor/close" | "/api/rt1/visitor/interrupt")
    {
        return None;
    }
    publication_qualification().err()
}

/// Only an explicit HTTPS DNS origin is eligible for cross-device invites.
/// No URL path, credentials, query, fragment, port, private IP or hostname
/// tricks may redirect visitor bearer credentials off the approved origin.
pub(super) fn verified(raw: &str) -> Option<String> {
    let host = raw.strip_prefix("https://")?;
    if host.len() > 253 || !host.contains('.') || host.ends_with('.') {
        return None;
    }
    if host
        .bytes()
        .any(|b| !(b.is_ascii_alphanumeric() || b == b'-' || b == b'.'))
    {
        return None;
    }
    if host.split('.').any(|label| {
        label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-')
    }) {
        return None;
    }
    let tld = host.rsplit('.').next()?;
    if !tld.bytes().all(|b| b.is_ascii_alphabetic()) || tld.len() < 2 {
        return None;
    }
    if matches!(tld, "local" | "localhost" | "invalid" | "test" | "example") {
        return None;
    }
    // Do not accept URL-like aliases or silently normalize a different
    // authority. DNS may vary by deployment; TLS and server_name are external.
    Some(raw.to_owned())
}

#[cfg(test)]
mod rt1_public_visitor_origin_tests {
    use super::verified;

    #[test]
    fn local_test_publication_remains_allowed_but_real_https_fails_closed() {
        use super::publication_qualification_for;
        assert_eq!(publication_qualification_for(None), Ok(()));
        assert_eq!(
            publication_qualification_for(Some("https://persona.example.org")),
            Err("RT1_PUBLIC_RELEASE_QUALIFICATION_REQUIRED"),
        );
    }

    #[test]
    fn public_release_does_not_claim_completed_rights_or_budget() {
        // Explicitly documents the intentionally unimplemented release gate.
        // Activation requires reviewed runtime authority, not a feature toggle.
        let rights: serde_json::Value =
            serde_json::from_str(include_str!("../../../contracts/rt1-consent-rights-0.1.json")).unwrap();
        let budget: serde_json::Value =
            serde_json::from_str(include_str!("../../../contracts/rt1-cost-budget-0.1.json")).unwrap();
        assert_eq!(rights["durable_consent_rights_state_implemented"], false);
        assert_eq!(budget["hard_session_budget_runtime_implemented"], false);
        assert_eq!(budget["prepublication_estimate_runtime_implemented"], false);
    }

    #[test]
    fn only_exact_public_https_dns_origin_is_admitted() {
        for value in ["https://persona.example.org", "https://vpr.example.co.uk"] {
            assert_eq!(verified(value).as_deref(), Some(value));
        }
        for value in [
            "",
            "http://persona.example.org",
            "https://localhost",
            "https://127.0.0.1",
            "https://persona.local",
            "https://persona.example.invalid",
            "https://persona.example.test",
            "https://persona.example.org:443",
            "https://persona.example.org/",
            "https://persona.example.org/rt1/visit",
            "https://persona.example.org?next=other",
            "https://persona.example.org#invite=secret",
            "https://me@persona.example.org",
            "https://persona.example.org@evil.tld",
            "https://persona.example.org.evil.tld/path",
            "https://-persona.example.org",
            "https://persona..org",
            "https://persona.example.org\\",
            "https://persona.example.org\nX-Foo: injected",
            "HTTPS://persona.example.org",
        ] {
            assert!(verified(value).is_none(), "{value:?}");
        }
    }
}
