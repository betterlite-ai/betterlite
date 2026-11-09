use std::collections::VecDeque;
use std::error::Error as _;
use std::sync::Mutex;

use betterlite_http::proxy::transport_error::TransportError;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::sap_auth::credentials::Credentials;
use crate::sap_auth::error::SapAuthError;
use crate::sap_auth::token::{
    ClientIdentity, TokenCache, TokenTransportError, build_token_request, parse_token_response,
};

use super::support::{FixedClock, ScriptedTransport, at, base_credentials};

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn exposed(value: Option<&SecretString>) -> Option<&str> {
    value.map(SecretString::expose_secret)
}

fn secret_credentials() -> Credentials {
    let mut credentials = base_credentials();
    credentials.client_secret = Some(secret("secret"));
    credentials
}

#[test]
fn the_token_request_carries_the_secret_form() {
    let request = build_token_request(&secret_credentials()).unwrap();
    assert_eq!(request.url, "https://auth.example.com/oauth/token");
    assert_eq!(request.form.grant_type, "client_credentials");
    assert_eq!(request.form.client_id, "client-id");
    assert_eq!(exposed(request.form.client_secret.as_ref()), Some("secret"));
    assert!(matches!(request.identity, ClientIdentity::None));
}

#[test]
fn the_secret_wins_over_certificate_identities() {
    let mut credentials = secret_credentials();
    credentials.cert_str = Some("cert".to_owned());
    credentials.key_str = Some(secret("key"));
    credentials.cert_file_path = Some("cert.pem".to_owned());
    credentials.key_file_path = Some("key.pem".to_owned());
    let request = build_token_request(&credentials).unwrap();
    assert_eq!(exposed(request.form.client_secret.as_ref()), Some("secret"));
    assert!(matches!(request.identity, ClientIdentity::None));
}

#[test]
fn an_empty_client_secret_is_left_out_and_does_not_win() {
    let mut credentials = base_credentials();
    credentials.client_secret = Some(secret(""));
    credentials.cert_str = Some("cert".to_owned());
    credentials.key_str = Some(secret("key"));
    let request = build_token_request(&credentials).unwrap();
    assert!(request.form.client_secret.is_none());
    assert!(matches!(request.identity, ClientIdentity::Pem { .. }));
}

#[test]
fn pem_strings_become_an_in_memory_identity() {
    let mut credentials = base_credentials();
    credentials.cert_str = Some("cert\\nline".to_owned());
    credentials.key_str = Some(secret("key\\nline"));
    let request = build_token_request(&credentials).unwrap();
    let ClientIdentity::Pem { cert, key } = request.identity else {
        panic!("expected a PEM identity");
    };
    assert_eq!(cert, "cert\nline");
    assert_eq!(key.expose_secret(), "key\nline");
}

#[test]
fn file_paths_become_a_file_identity() {
    let mut credentials = base_credentials();
    credentials.cert_file_path = Some("cert.pem".to_owned());
    credentials.key_file_path = Some("key.pem".to_owned());
    let request = build_token_request(&credentials).unwrap();
    let ClientIdentity::Files {
        cert_path,
        key_path,
    } = request.identity
    else {
        panic!("expected a file identity");
    };
    assert_eq!(cert_path, "cert.pem");
    assert_eq!(key_path, "key.pem");
}

#[test]
fn a_pem_pair_wins_over_file_paths() {
    let mut credentials = base_credentials();
    credentials.cert_str = Some("cert".to_owned());
    credentials.key_str = Some(secret("key"));
    credentials.cert_file_path = Some("cert.pem".to_owned());
    credentials.key_file_path = Some("key.pem".to_owned());
    let request = build_token_request(&credentials).unwrap();
    assert!(matches!(request.identity, ClientIdentity::Pem { .. }));
}

#[test]
fn a_pem_pair_with_an_empty_key_is_not_an_identity() {
    let mut credentials = base_credentials();
    credentials.cert_str = Some("cert".to_owned());
    credentials.key_str = Some(secret(""));
    let error = build_token_request(&credentials).unwrap_err();
    assert!(matches!(error, SapAuthError::IncompleteCredentials));
}

#[test]
fn a_token_request_without_an_authentication_mode_is_an_error() {
    let error = build_token_request(&base_credentials()).unwrap_err();
    assert!(matches!(error, SapAuthError::IncompleteCredentials));
}

#[test]
fn a_token_request_without_an_auth_url_is_an_error() {
    let mut credentials = secret_credentials();
    credentials.auth_url = None;
    let error = build_token_request(&credentials).unwrap_err();
    assert!(matches!(error, SapAuthError::MissingCredentials));
}

#[test]
fn a_token_request_with_an_empty_auth_url_is_an_error() {
    let mut credentials = secret_credentials();
    credentials.auth_url = Some(String::new());
    let error = build_token_request(&credentials).unwrap_err();
    assert!(matches!(error, SapAuthError::MissingCredentials));
}

#[test]
fn a_token_request_without_a_client_id_is_an_error() {
    let mut credentials = secret_credentials();
    credentials.client_id = None;
    let error = build_token_request(&credentials).unwrap_err();
    assert!(matches!(error, SapAuthError::MissingCredentials));
}

#[test]
fn a_token_response_defaults_the_expiry_to_one_hour() {
    let token = parse_token_response(r#"{"access_token": "abc"}"#, at(1_700_000_000)).unwrap();
    assert_eq!(token.header.expose_secret(), "Bearer abc");
    assert_eq!(token.expires_at, at(1_700_003_600));
}

#[test]
fn a_bearer_token_does_not_print_itself() {
    let token =
        parse_token_response(r#"{"access_token": "very-hidden"}"#, at(1_700_000_000)).unwrap();
    assert!(!format!("{token:?}").contains("very-hidden"));
}

#[test]
fn a_token_response_uses_expires_in() {
    let token = parse_token_response(
        r#"{"access_token": "abc", "expires_in": 120}"#,
        at(1_700_000_000),
    )
    .unwrap();
    assert_eq!(token.expires_at, at(1_700_000_120));
}

#[test]
fn a_negative_expires_in_puts_the_expiry_in_the_past() {
    let token = parse_token_response(
        r#"{"access_token": "abc", "expires_in": -5}"#,
        at(1_700_000_000),
    )
    .unwrap();
    assert_eq!(token.expires_at, at(1_699_999_995));
}

#[rstest]
#[case::missing_access_token(r#"{"expires_in": 60}"#, r#"{"expires_in": 60}"#)]
#[case::not_json("not json", "not json")]
#[case::json_list("[]", "[]")]
#[case::json_null("null", "null")]
#[case::text_access_token(r#"{"access_token": 7}"#, r#"{"access_token": 7}"#)]
#[case::string_expires_in(
    r#"{"access_token": "a", "expires_in": "3600"}"#,
    r#"{"access_token": "<redacted>", "expires_in": "3600"}"#
)]
#[case::float_expires_in(
    r#"{"access_token": "a", "expires_in": 3600.5}"#,
    r#"{"access_token": "<redacted>", "expires_in": 3600.5}"#
)]
#[case::null_expires_in(
    r#"{"access_token": "a", "expires_in": null}"#,
    r#"{"access_token": "<redacted>", "expires_in": null}"#
)]
#[case::bool_expires_in(
    r#"{"access_token": "a", "expires_in": true}"#,
    r#"{"access_token": "<redacted>", "expires_in": true}"#
)]
#[case::huge_expires_in(
    r#"{"access_token": "a", "expires_in": 9223372036854775807}"#,
    r#"{"access_token": "<redacted>", "expires_in": 9223372036854775807}"#
)]
#[case::escaped_token(
    r#"{"access_token": "abc\/def+ghi", "expires_in": 99999999999999}"#,
    r#"{"access_token": "<redacted>", "expires_in": 99999999999999}"#
)]
#[case::out_of_range_expiry(
    r#"{"access_token": "tok", "expires_in": 1e999}"#,
    r#"{"access_token": "<redacted>", "expires_in": 1e999}"#
)]
#[case::truncated_token_body(
    r#"{"access_token": "tok", "expires_in": 36"#,
    "token response names access_token but is not valid JSON, body withheld"
)]
#[case::trailing_garbage_token_body(
    r#"{"access_token": "tok", "expires_in": 3600} trailing"#,
    "token response names access_token but is not valid JSON, body withheld"
)]
#[case::invalid_client_error(r#"{"error":"invalid_client"}"#, r#"{"error":"invalid_client"}"#)]
fn a_bad_token_response_fails_with_its_body(#[case] body: &str, #[case] text: &str) {
    let error = parse_token_response(body, at(1_700_000_000)).unwrap_err();
    assert_eq!(error.to_string(), format!("Token request failed: {text}"));
}

#[test]
fn an_overflowing_expiry_does_not_carry_the_access_token_into_the_text() {
    let body = r#"{"access_token": "tok", "expires_in": 99999999999999}"#;
    let error = parse_token_response(body, at(1_700_000_000)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Token request failed: {\"access_token\": \"<redacted>\", \"expires_in\": 99999999999999}"
    );
    assert!(!error.to_string().contains("\"tok\""));
}

#[test]
fn a_bad_token_response_without_an_access_token_keeps_its_text() {
    let body = r#"{"expires_in": 99999999999999}"#;
    let error = parse_token_response(body, at(1_700_000_000)).unwrap_err();
    assert_eq!(error.to_string(), format!("Token request failed: {body}"));
}

#[test]
fn the_token_cache_reuses_a_token_whose_life_is_outside_the_buffer() {
    let cache = TokenCache::new(&secret_credentials()).unwrap();
    let clock = FixedClock {
        now: at(1_700_000_000),
    };
    let transport = ScriptedTransport {
        responses: Mutex::new(VecDeque::from([
            Ok(r#"{"access_token": "one", "expires_in": 7200}"#.to_owned()),
            Ok(r#"{"access_token": "two", "expires_in": 7200}"#.to_owned()),
        ])),
        requests: Mutex::new(Vec::new()),
    };
    assert_eq!(
        cache.token(&clock, &transport).unwrap().expose_secret(),
        "Bearer one"
    );
    assert_eq!(
        cache.token(&clock, &transport).unwrap().expose_secret(),
        "Bearer one"
    );
    assert_eq!(transport.request_count(), 1);
    assert_eq!(
        transport.request_urls(),
        vec!["https://auth.example.com/oauth/token".to_owned()]
    );
}

#[test]
fn the_token_cache_keeps_a_token_with_exactly_the_buffer_left() {
    let cache = TokenCache::new(&secret_credentials()).unwrap();
    let transport = ScriptedTransport {
        responses: Mutex::new(VecDeque::from([
            Ok(r#"{"access_token": "one", "expires_in": 7200}"#.to_owned()),
            Ok(r#"{"access_token": "two", "expires_in": 7200}"#.to_owned()),
        ])),
        requests: Mutex::new(Vec::new()),
    };
    let early = FixedClock {
        now: at(1_700_000_000),
    };
    let boundary = FixedClock {
        now: at(1_700_003_600),
    };
    assert_eq!(
        cache.token(&early, &transport).unwrap().expose_secret(),
        "Bearer one"
    );
    assert_eq!(
        cache.token(&boundary, &transport).unwrap().expose_secret(),
        "Bearer one"
    );
    assert_eq!(transport.request_count(), 1);
}

#[test]
fn the_token_cache_refetches_once_the_life_drops_under_the_buffer() {
    let cache = TokenCache::new(&secret_credentials()).unwrap();
    let transport = ScriptedTransport {
        responses: Mutex::new(VecDeque::from([
            Ok(r#"{"access_token": "one", "expires_in": 7200}"#.to_owned()),
            Ok(r#"{"access_token": "two", "expires_in": 7200}"#.to_owned()),
        ])),
        requests: Mutex::new(Vec::new()),
    };
    let early = FixedClock {
        now: at(1_700_000_000),
    };
    let later = FixedClock {
        now: at(1_700_003_601),
    };
    assert_eq!(
        cache.token(&early, &transport).unwrap().expose_secret(),
        "Bearer one"
    );
    assert_eq!(
        cache.token(&later, &transport).unwrap().expose_secret(),
        "Bearer two"
    );
    assert_eq!(transport.request_count(), 2);
}

#[test]
fn a_one_hour_token_is_refetched_after_any_time_passes() {
    let cache = TokenCache::new(&secret_credentials()).unwrap();
    let transport = ScriptedTransport {
        responses: Mutex::new(VecDeque::from([
            Ok(r#"{"access_token": "one"}"#.to_owned()),
            Ok(r#"{"access_token": "two"}"#.to_owned()),
        ])),
        requests: Mutex::new(Vec::new()),
    };
    let early = FixedClock {
        now: at(1_700_000_000),
    };
    let one_second_later = FixedClock {
        now: at(1_700_000_001),
    };
    assert_eq!(
        cache.token(&early, &transport).unwrap().expose_secret(),
        "Bearer one"
    );
    assert_eq!(
        cache
            .token(&one_second_later, &transport)
            .unwrap()
            .expose_secret(),
        "Bearer two"
    );
    assert_eq!(transport.request_count(), 2);
}

#[test]
fn the_token_cache_reports_a_transport_failure_with_its_text() {
    let cache = TokenCache::new(&secret_credentials()).unwrap();
    let clock = FixedClock {
        now: at(1_700_000_000),
    };
    let transport = ScriptedTransport {
        responses: Mutex::new(VecDeque::from([Err(TokenTransportError::Message(
            "connection refused".to_owned(),
        ))])),
        requests: Mutex::new(Vec::new()),
    };
    let error = cache.token(&clock, &transport).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Token request failed: connection refused"
    );
}

#[test]
fn the_token_cache_reports_a_parse_failure_with_the_body() {
    let cache = TokenCache::new(&secret_credentials()).unwrap();
    let clock = FixedClock {
        now: at(1_700_000_000),
    };
    let transport = ScriptedTransport {
        responses: Mutex::new(VecDeque::from([Ok("not json".to_owned())])),
        requests: Mutex::new(Vec::new()),
    };
    let error = cache.token(&clock, &transport).unwrap_err();
    assert_eq!(error.to_string(), "Token request failed: not json");
}

#[test]
fn a_parse_failure_keeps_the_serde_error_as_its_source() {
    let error = parse_token_response("not json", at(1_700_000_000)).unwrap_err();
    let failure = error.source().unwrap();
    assert!(
        failure
            .source()
            .unwrap()
            .downcast_ref::<serde_json::Error>()
            .is_some()
    );
}

#[test]
fn a_transport_failure_keeps_the_transport_error_as_its_source() {
    let cache = TokenCache::new(&secret_credentials()).unwrap();
    let clock = FixedClock {
        now: at(1_700_000_000),
    };
    let transport = ScriptedTransport {
        responses: Mutex::new(VecDeque::from([Err(TokenTransportError::Transport(
            Box::new(TransportError::Closed),
        ))])),
        requests: Mutex::new(Vec::new()),
    };
    let error = cache.token(&clock, &transport).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Token request failed: the client is shutting down and takes no new requests"
    );
    let failure = error.source().unwrap();
    let transport_failure = failure.source().unwrap();
    assert!(matches!(
        transport_failure.downcast_ref::<TokenTransportError>(),
        Some(TokenTransportError::Transport(_))
    ));
    assert!(
        transport_failure
            .source()
            .unwrap()
            .downcast_ref::<Box<TransportError>>()
            .is_some()
    );
}

#[test]
fn the_token_cache_validates_the_credentials_once() {
    let error = TokenCache::new(&base_credentials()).unwrap_err();
    assert!(matches!(error, SapAuthError::IncompleteCredentials));
}
