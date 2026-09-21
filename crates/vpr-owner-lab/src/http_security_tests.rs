use super::{CONTENT_SECURITY_POLICY, valid_host_value, valid_origin_value};

#[test]
fn host_validation_rejects_dns_rebinding_shapes() {
    for accepted in ["127.0.0.1:8787", "localhost:8787"] {
        assert!(
            valid_host_value(accepted, 8787),
            "expected accepted host: {accepted}"
        );
    }
    for denied in [
        "127.0.0.1",
        "localhost",
        "localhost:9999",
        "localhost.evil.test",
        "127.0.0.1.evil.test",
        "evil.test",
        "0.0.0.0:8787",
        "[::1]:8787",
    ] {
        assert!(
            !valid_host_value(denied, 8787),
            "expected denied host: {denied}"
        );
    }
}

#[test]
fn post_origin_must_match_the_loopback_host_exactly() {
    assert!(valid_origin_value(
        "127.0.0.1:8787",
        "http://127.0.0.1:8787",
        8787
    ));
    assert!(valid_origin_value(
        "localhost:8787",
        "http://localhost:8787",
        8787
    ));
    assert!(!valid_origin_value(
        "127.0.0.1:8787",
        "http://localhost:8787",
        8787
    ));
    assert!(!valid_origin_value(
        "localhost:8787",
        "https://localhost:8787",
        8787
    ));
    assert!(!valid_origin_value(
        "localhost.evil.test",
        "http://localhost.evil.test",
        8787
    ));
}


#[test]
fn csp_allows_provider_neutral_secure_realtime_signal_fallbacks() {
    assert!(CONTENT_SECURITY_POLICY.contains("connect-src 'self' https: wss:"));
    assert!(!CONTENT_SECURITY_POLICY.contains("connect-src 'self' http:"));
}
