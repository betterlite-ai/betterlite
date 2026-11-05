#![cfg(test)]

use std::time::Duration;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::advance;

use super::authorization_blocking;
use crate::cyberark::cyberark_read::read;
use crate::cyberark::cyberark_write::write;
use crate::cyberark::error::CyberArkInitError;
use crate::cyberark::fake_transport::{
    AUTH_ROUTE, FakeTransport, Reply, TOKEN_HEADER, TOKEN_JSON, capture_logs, environment, keyed,
    scripted,
};
use crate::cyberark::transport::ClientCertificate;
use crate::cyberark::{CyberArkSettings, DEFAULT_REFRESH_SECONDS};

const SECRET_ROUTE: &str = "GET http://127.0.0.1:8080/secrets/default/variable/db";

#[test]
fn defaults_apply_when_only_the_api_key_is_set() {
    let settings =
        CyberArkSettings::from_environment(&environment(&[("CYBERARK_API_KEY", "key")])).unwrap();
    assert_eq!(settings.api_base, "http://127.0.0.1:8080");
    assert_eq!(settings.account, "default");
    assert_eq!(settings.username, "admin");
    assert_eq!(settings.api_key.expose_secret(), "key");
    assert_eq!(settings.client_certificate, None);
    assert!(settings.verify_tls);
    assert_eq!(
        settings.refresh_interval,
        Duration::from_secs(DEFAULT_REFRESH_SECONDS)
    );
}

#[test]
fn every_setting_is_read_from_the_environment() {
    let settings = CyberArkSettings::from_environment(&environment(&[
        ("CYBERARK_API_BASE", "https://conjur.example"),
        ("CYBERARK_ACCOUNT", "corp"),
        ("CYBERARK_USERNAME", "svc"),
        ("CYBERARK_API_KEY", "key"),
        ("CYBERARK_CLIENT_CERT", "/c.pem"),
        ("CYBERARK_CLIENT_KEY", "/k.pem"),
        ("CYBERARK_SSL_VERIFY", "false"),
        ("CYBERARK_REFRESH_INTERVAL", "42"),
    ]))
    .unwrap();
    assert_eq!(settings.api_base, "https://conjur.example");
    assert_eq!(settings.account, "corp");
    assert_eq!(settings.username, "svc");
    assert_eq!(
        settings.client_certificate,
        Some(ClientCertificate {
            certificate_path: "/c.pem".to_owned(),
            key_path: "/k.pem".to_owned(),
        })
    );
    assert!(!settings.verify_tls);
    assert_eq!(settings.refresh_interval, Duration::from_secs(42));
}

#[test]
fn empty_values_count_as_unset() {
    let settings = CyberArkSettings::from_environment(&environment(&[
        ("CYBERARK_API_KEY", "key"),
        ("CYBERARK_API_BASE", ""),
        ("CYBERARK_ACCOUNT", ""),
        ("CYBERARK_REFRESH_INTERVAL", ""),
    ]))
    .unwrap();
    assert_eq!(settings.api_base, "http://127.0.0.1:8080");
    assert_eq!(settings.account, "default");
    assert_eq!(
        settings.refresh_interval,
        Duration::from_secs(DEFAULT_REFRESH_SECONDS)
    );
}

#[rstest]
#[case::nothing(&[])]
#[case::empty_key(&[("CYBERARK_API_KEY", "")])]
#[case::certificate_only(&[("CYBERARK_CLIENT_CERT", "/c.pem")])]
#[case::key_only(&[("CYBERARK_CLIENT_KEY", "/k.pem")])]
fn initialization_requires_credentials(#[case] pairs: &[(&str, &str)]) {
    let error = CyberArkSettings::from_environment(&environment(pairs)).unwrap_err();
    assert!(matches!(error, CyberArkInitError::MissingCredentials));
}

#[test]
fn a_certificate_pair_replaces_the_api_key() {
    let settings = CyberArkSettings::from_environment(&environment(&[
        ("CYBERARK_CLIENT_CERT", "/c.pem"),
        ("CYBERARK_CLIENT_KEY", "/k.pem"),
    ]))
    .unwrap();
    assert!(settings.client_certificate.is_some());
    assert_eq!(settings.api_key.expose_secret(), "");
}

#[test]
fn refresh_interval_must_be_a_number() {
    let error = CyberArkSettings::from_environment(&environment(&[
        ("CYBERARK_API_KEY", "key"),
        ("CYBERARK_REFRESH_INTERVAL", "soon"),
    ]))
    .unwrap_err();
    assert!(matches!(error, CyberArkInitError::RefreshInterval(_)));
}

#[rstest]
#[case::unset(None, true)]
#[case::true_text(Some("true"), true)]
#[case::false_text(Some("false"), false)]
#[case::padded_mixed_case(Some(" False "), false)]
#[case::unrecognized(Some("maybe"), true)]
#[case::empty(Some(""), true)]
fn ssl_verification_defaults_to_true(#[case] text: Option<&str>, #[case] verify: bool) {
    let mut pairs = vec![("CYBERARK_API_KEY", "key")];
    pairs.extend(text.map(|value| ("CYBERARK_SSL_VERIFY", value)));
    let settings = CyberArkSettings::from_environment(&environment(&pairs)).unwrap();
    assert_eq!(settings.verify_tls, verify);
}

#[test]
fn disabled_verification_warns_once_at_construction() {
    let (logs, _guard) = capture_logs();
    let _manager = scripted(
        &[
            ("CYBERARK_API_KEY", "key"),
            ("CYBERARK_SSL_VERIFY", "false"),
        ],
        &[],
    );
    let text = logs.text();
    assert_eq!(
        text.matches("WARN CyberArk SSL verification is disabled")
            .count(),
        1
    );
    let (quiet, _quiet_guard) = capture_logs();
    let _verified = keyed(&[]);
    assert_eq!(quiet.text(), "");
}

#[tokio::test(start_paused = true)]
async fn api_key_login_posts_the_key_and_base64_encodes_the_token() {
    let (manager, transport) = keyed(&[(SECRET_ROUTE, vec![Reply::Status(200, "v")])]);
    read(&manager, "db").await.unwrap();
    let seen = transport.recorded();
    let login = seen.first().unwrap();
    assert_eq!(login.route, AUTH_ROUTE);
    assert_eq!(login.body.as_deref(), Some("key"));
    assert_eq!(login.client_certificate, None);
    assert_eq!(login.authorization, None);
    assert!(login.verify_tls);
    assert_eq!(seen[1].authorization.as_deref(), Some(TOKEN_HEADER));
}

#[tokio::test(start_paused = true)]
async fn certificate_login_sends_the_pair_and_an_empty_body() {
    let (manager, transport) = scripted(
        &[
            ("CYBERARK_CLIENT_CERT", "/c.pem"),
            ("CYBERARK_CLIENT_KEY", "/k.pem"),
            ("CYBERARK_SSL_VERIFY", "false"),
        ],
        &[(AUTH_ROUTE, vec![Reply::Status(200, TOKEN_JSON)])],
    );
    read(&manager, "db").await;
    let login = transport.recorded().into_iter().next().unwrap();
    assert_eq!(login.body.as_deref(), Some(""));
    assert!(!login.verify_tls);
    assert_eq!(
        login.client_certificate,
        Some(ClientCertificate {
            certificate_path: "/c.pem".to_owned(),
            key_path: "/k.pem".to_owned(),
        })
    );
}

#[tokio::test(start_paused = true)]
async fn the_token_is_cached_for_the_refresh_interval() {
    let (manager, transport) = scripted(
        &[
            ("CYBERARK_API_KEY", "key"),
            ("CYBERARK_REFRESH_INTERVAL", "10"),
        ],
        &[(AUTH_ROUTE, vec![Reply::Status(200, TOKEN_JSON)])],
    );
    read(&manager, "one").await;
    read(&manager, "two").await;
    advance(Duration::from_secs(10)).await;
    read(&manager, "three").await;
    assert_eq!(count_logins(&transport), 1);
    advance(Duration::from_millis(1)).await;
    read(&manager, "four").await;
    assert_eq!(count_logins(&transport), 2);
}

#[tokio::test(start_paused = true)]
async fn a_rejected_login_is_reported_and_not_cached() {
    let (manager, transport) = scripted(
        &[("CYBERARK_API_KEY", "key")],
        &[(AUTH_ROUTE, vec![Reply::Status(401, "")])],
    );
    let first = write(&manager, "db", &SecretString::from("v")).await;
    assert!(
        first
            .message
            .starts_with("Could not authenticate to CyberArk Conjur: ")
    );
    assert!(first.message.contains("401"));
    assert_eq!(count_logins(&transport), 2);
    write(&manager, "db", &SecretString::from("v")).await;
    assert_eq!(count_logins(&transport), 4);
}

#[tokio::test(start_paused = true)]
async fn a_login_transport_failure_is_reported() {
    let (manager, _transport) = scripted(
        &[("CYBERARK_API_KEY", "key")],
        &[(AUTH_ROUTE, vec![Reply::Failure])],
    );
    let outcome = write(&manager, "db", &SecretString::from("v")).await;
    assert!(outcome.message.contains("connection refused"));
}

#[test]
fn blocking_login_uses_the_blocking_transport() {
    let (manager, transport) = keyed(&[(SECRET_ROUTE, vec![Reply::Status(200, "v")])]);
    authorization_blocking(&manager).unwrap();
    assert!(transport.recorded().iter().all(|seen| seen.blocking));
}

fn count_logins(transport: &FakeTransport) -> usize {
    transport
        .routes_hit()
        .iter()
        .filter(|route| route.as_str() == AUTH_ROUTE)
        .count()
}
