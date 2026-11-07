use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::tls::{TlsPolicy, Verification};
use rstest::rstest;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::client::sessions::factory::{AuthSource, RedirectGuard, TlsSettings, TlsVerification};
use crate::client::sessions::transport::TransportType;
use crate::credentials::{AuthValue, McpAuthType};
use secrecy::SecretString;

use super::fixture::{FakeRuntime, Plan, PresenceAuth, build_client, make_config};

#[rstest]
#[case::explicit(true, true, true, AuthSource::Explicit)]
#[case::resolved(false, true, true, AuthSource::Resolved)]
#[case::aws(false, false, true, AuthSource::Aws)]
#[case::none(false, false, false, AuthSource::None)]
#[tokio::test]
async fn factory_auth_precedence_is_explicit_then_resolved_then_aws(
    #[case] explicit: bool,
    #[case] resolved: bool,
    #[case] aws: bool,
    #[case] expected: AuthSource,
) {
    let client = build_client(
        FakeRuntime::new(Plan::default()),
        make_config(|value| {
            value.resolved_auth = resolved.then_some(PresenceAuth { header_name: None });
            value.aws_auth = aws.then_some(PresenceAuth { header_name: None });
        }),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let headers = BTreeMap::new();
    let config = client.create_http_client_config(&headers, Duration::from_secs(60), explicit);
    assert_eq!(config.auth, expected);
}

#[test]
fn factory_debug_output_names_the_headers_without_printing_their_values() {
    let client = build_client(
        FakeRuntime::new(Plan::default()),
        make_config(|value| {
            value.auth_type = McpAuthType::ApiKey;
            value.auth_value = Some(AuthValue::Text(SecretString::from("sk-secret".to_owned())));
        }),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let headers = BTreeMap::from([("X-API-Key".to_owned(), "sk-secret".to_owned())]);
    let config = client.create_http_client_config(&headers, Duration::from_secs(60), false);
    let rendered = format!("{config:?}");
    assert!(rendered.contains("X-API-Key"));
    assert!(!rendered.contains("sk-secret"));
}

#[tokio::test]
async fn factory_attaches_the_resolved_tls_settings() {
    let policy = TlsPolicy {
        verification: Verification::CaFile("/etc/ca.pem".to_owned()),
        ca_file: Some("/etc/ca.pem".to_owned()),
        certificate: None,
        http2: true,
        ecdh_curve: None,
        user_agent: "betterlite".to_owned(),
    };
    let client = build_client(
        FakeRuntime::new(Plan::default()),
        make_config(|value| value.tls = policy),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let config = client.create_http_client_config(&BTreeMap::new(), Duration::from_secs(60), false);
    assert_eq!(
        config.tls,
        TlsSettings {
            verification: TlsVerification::CaFile,
            ca_file: Some("/etc/ca.pem".to_owned()),
        }
    );
    assert!(config.follow_redirects);
    assert!(config.capture_error_responses);
}

fn guarded_client(server_url: &str, auth_header_name: Option<&str>) -> RedirectGuard {
    let client = build_client(
        FakeRuntime::new(Plan::default()),
        make_config(|value| {
            value.server_url = server_url.to_owned();
            value.transport_type = TransportType::Http;
            value.auth_type = McpAuthType::ApiKey;
            value.auth_value = Some(AuthValue::Text(SecretString::from("key".to_owned())));
            value.auth_header_name = auth_header_name.map(str::to_owned);
        }),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    client
        .create_http_client_config(&BTreeMap::new(), Duration::from_secs(60), false)
        .redirect_guard
        .unwrap()
}

fn applied(guard: &RedirectGuard, url: &str) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::from([
        ("X-API-Key".to_owned(), "key".to_owned()),
        ("X-Other".to_owned(), "kept".to_owned()),
    ]);
    guard.apply(url, &mut headers);
    headers
}

#[test]
fn redirect_guard_removes_the_credential_when_the_origin_changes() {
    let guard = guarded_client("https://mcp.example.com/mcp", Some("X-API-Key"));
    assert_eq!(guard.configured_url, "https://mcp.example.com/mcp");
    assert_eq!(guard.slot, "X-API-Key");

    let same_origin = applied(&guard, "https://mcp.example.com/other");
    assert_eq!(
        same_origin.get("X-API-Key").map(String::as_str),
        Some("key")
    );

    let other_host = applied(&guard, "https://evil.example.net/mcp");
    assert!(!other_host.contains_key("X-API-Key"));
    assert_eq!(other_host.get("X-Other").map(String::as_str), Some("kept"));

    let other_port = applied(&guard, "https://mcp.example.com:8443/mcp");
    assert!(!other_port.contains_key("X-API-Key"));

    let downgrade = applied(&guard, "http://mcp.example.com/mcp");
    assert!(!downgrade.contains_key("X-API-Key"));
}

#[test]
fn redirect_guard_keeps_an_http_to_https_upgrade_of_the_same_host() {
    let guard = guarded_client("http://mcp.example.com/mcp", Some("X-API-Key"));
    let upgraded = applied(&guard, "https://mcp.example.com/mcp");
    assert_eq!(upgraded.get("X-API-Key").map(String::as_str), Some("key"));
}

#[test]
fn redirect_guard_is_absent_without_a_configured_slot() {
    let client = build_client(
        FakeRuntime::new(Plan::default()),
        make_config(|value| {
            value.auth_type = McpAuthType::BearerToken;
            value.auth_value = Some(AuthValue::Text(SecretString::from("token".to_owned())));
        }),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let config = client.create_http_client_config(&BTreeMap::new(), Duration::from_secs(60), false);
    assert!(config.redirect_guard.is_none());
}

#[test]
fn redirect_guard_is_absent_when_the_credential_uses_authorization() {
    let client = build_client(
        FakeRuntime::new(Plan::default()),
        make_config(|value| {
            value.auth_type = McpAuthType::BearerToken;
            value.auth_value = Some(AuthValue::Text(SecretString::from("token".to_owned())));
            value.auth_header_name = Some("Authorization".to_owned());
        }),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let config = client.create_http_client_config(&BTreeMap::new(), Duration::from_secs(60), false);
    assert!(config.redirect_guard.is_none());
}

#[test]
fn redirect_guard_is_absent_for_an_empty_server_url() {
    let client = build_client(
        FakeRuntime::new(Plan::default()),
        make_config(|value| {
            value.server_url = String::new();
            value.auth_type = McpAuthType::ApiKey;
            value.auth_value = Some(AuthValue::Text(SecretString::from("key".to_owned())));
            value.auth_header_name = Some("X-API-Key".to_owned());
        }),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let config = client.create_http_client_config(&BTreeMap::new(), Duration::from_secs(60), false);
    assert!(config.redirect_guard.is_none());
}

#[test]
fn redirect_guard_rejects_an_unparsable_configuration_url() {
    let guard = RedirectGuard::new("not a url", Some("X-API-Key")).unwrap();
    let mut headers = BTreeMap::from([("X-API-Key".to_owned(), "key".to_owned())]);
    guard.apply("https://mcp.example.com/mcp", &mut headers);
    assert!(headers.is_empty());
}
