#![cfg(test)]

use super::{strip_userinfo, uses_tls};

#[test]
fn redis_tls_is_detected_from_the_rediss_scheme() {
    assert!(uses_tls(false, None, Some("rediss://cache.internal:6379")));
    assert!(!uses_tls(false, None, Some("redis://cache.internal:6379")));
}

#[test]
fn redis_tls_falls_back_to_ssl_when_there_is_no_url() {
    assert!(uses_tls(false, Some(true), None));
    assert!(!uses_tls(false, Some(false), None));
    assert!(!uses_tls(false, None, None));
}

#[test]
fn redis_tls_prefers_ssl_when_startup_nodes_are_present_even_with_a_rediss_url() {
    assert!(!uses_tls(
        true,
        Some(false),
        Some("rediss://cache.internal:6379")
    ));
    assert!(uses_tls(
        true,
        Some(true),
        Some("redis://cache.internal:6379")
    ));
}

#[test]
fn redis_credential_strip_userinfo_removes_username_and_password() {
    let stripped = strip_userinfo("redis://user:pass@cache.internal:6379/0")
        .expect("a well formed url strips");
    assert_eq!(stripped, "redis://cache.internal:6379/0");
}

#[test]
fn redis_credential_strip_userinfo_rejects_a_malformed_url() {
    strip_userinfo("not a url").unwrap_err();
}
