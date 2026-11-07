#![cfg(test)]

use std::fmt::Debug;
use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret as _;
use tokio::time::advance;

use crate::cyberark::transport::ClientCertificate;
use crate::vault::error::{LoginMethod, VaultError};
use crate::vault::tests::{
    APPROLE_ROUTE, LOGIN_BODY, Refused, Reply, SECOND_LOGIN_BODY, TLS_ROUTE, answered,
    capture_logs, scripted,
};
use crate::vault::transport::{MAX_RESPONSE_BYTES, VaultMethod};

const APPROLE: &[(&str, &str)] = &[
    ("HCP_VAULT_APPROLE_ROLE_ID", "role-id"),
    ("HCP_VAULT_APPROLE_SECRET_ID", "secret-id"),
];

const TLS: &[(&str, &str)] = &[
    ("HCP_VAULT_CLIENT_CERT", "/certs/client.crt"),
    ("HCP_VAULT_CLIENT_KEY", "/certs/client.key"),
    ("HCP_VAULT_CERT_ROLE", "vault-role"),
];

#[tokio::test]
async fn approle_login_sends_the_login_request() {
    let (manager, transport) = scripted(
        &[
            ("HCP_VAULT_NAMESPACE", "admin"),
            ("HCP_VAULT_APPROLE_ROLE_ID", "role-id"),
            ("HCP_VAULT_APPROLE_SECRET_ID", "secret-id"),
        ],
        &[(APPROLE_ROUTE, vec![answered(200, LOGIN_BODY)])],
    );
    let token = manager.request_token().await.unwrap();
    assert_eq!(token.expose_secret(), "hvs.first");
    let recorded = transport.recorded();
    assert_eq!(recorded.len(), 1);
    let request = &recorded[0];
    assert_eq!(request.method, VaultMethod::Post);
    assert_eq!(request.url, "http://127.0.0.1:8200/v1/auth/approle/login");
    assert_eq!(request.namespace.as_deref(), Some("admin"));
    assert_eq!(request.token, None);
    assert_eq!(request.client_certificate, None);
    assert_eq!(request.timeout, Duration::from_secs(600));
    assert!(request.follow_redirects);
    assert_eq!(request.max_response_bytes, MAX_RESPONSE_BYTES);
    assert!(!request.blocking);
    assert_eq!(
        request.body.as_deref(),
        Some("{\"role_id\":\"role-id\",\"secret_id\":\"secret-id\"}")
    );
}

#[tokio::test]
async fn approle_login_blocking_uses_the_blocking_transport() {
    let (manager, transport) =
        scripted(APPROLE, &[(APPROLE_ROUTE, vec![answered(200, LOGIN_BODY)])]);
    let token = manager.request_token_blocking().unwrap();
    assert_eq!(token.expose_secret(), "hvs.first");
    let recorded = transport.recorded();
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].blocking);
}

#[tokio::test(start_paused = true)]
async fn approle_token_is_reused_until_the_lease_ends() {
    let first = "{\"auth\":{\"client_token\":\"hvs.first\",\"lease_duration\":10}}";
    let (manager, transport) = scripted(
        APPROLE,
        &[(
            APPROLE_ROUTE,
            vec![answered(200, first), answered(200, SECOND_LOGIN_BODY)],
        )],
    );
    assert_eq!(
        manager.request_token().await.unwrap().expose_secret(),
        "hvs.first"
    );
    assert_eq!(
        manager.request_token().await.unwrap().expose_secret(),
        "hvs.first"
    );
    assert_eq!(transport.recorded().len(), 1);
    advance(Duration::from_secs(10)).await;
    assert_eq!(
        manager.request_token().await.unwrap().expose_secret(),
        "hvs.first"
    );
    assert_eq!(transport.recorded().len(), 1);
    advance(Duration::from_millis(1)).await;
    assert_eq!(
        manager.request_token().await.unwrap().expose_secret(),
        "hvs.second"
    );
    assert_eq!(transport.recorded().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_blocking_request_reuses_the_token_the_async_path_cached() {
    let first = "{\"auth\":{\"client_token\":\"hvs.first\",\"lease_duration\":60}}";
    let (manager, transport) = scripted(APPROLE, &[(APPROLE_ROUTE, vec![answered(200, first)])]);
    manager.request_token().await.unwrap();
    assert_eq!(
        manager.request_token_blocking().unwrap().expose_secret(),
        "hvs.first"
    );
    assert_eq!(transport.recorded().len(), 1);
}

#[tokio::test]
async fn tls_login_sends_the_certificate_pair_and_the_role() {
    let (manager, transport) = scripted(TLS, &[(TLS_ROUTE, vec![answered(200, LOGIN_BODY)])]);
    let token = manager.request_token().await.unwrap();
    assert_eq!(token.expose_secret(), "hvs.first");
    let recorded = transport.recorded();
    assert_eq!(recorded.len(), 1);
    let request = &recorded[0];
    assert_eq!(request.method, VaultMethod::Post);
    assert_eq!(request.url, "http://127.0.0.1:8200/v1/auth/cert/login");
    assert_eq!(request.namespace, None);
    assert_eq!(request.token, None);
    assert_eq!(request.timeout, Duration::from_secs(5));
    assert!(!request.follow_redirects);
    assert_eq!(
        request
            .client_certificate
            .as_ref()
            .map(|pair| (pair.certificate_path.as_str(), pair.key_path.as_str())),
        Some(("/certs/client.crt", "/certs/client.key"))
    );
    assert_eq!(request.body.as_deref(), Some("{\"name\":\"vault-role\"}"));
}

#[tokio::test]
async fn tls_login_sends_a_null_role_when_unset() {
    let (manager, transport) = scripted(
        &[
            ("HCP_VAULT_CLIENT_CERT", "/certs/client.crt"),
            ("HCP_VAULT_CLIENT_KEY", "/certs/client.key"),
        ],
        &[(TLS_ROUTE, vec![answered(200, LOGIN_BODY)])],
    );
    manager.request_token().await.unwrap();
    assert_eq!(
        transport.recorded()[0].body.as_deref(),
        Some("{\"name\":null}")
    );
}

#[tokio::test]
async fn tls_login_runs_on_every_call() {
    let (manager, transport) = scripted(
        TLS,
        &[(
            TLS_ROUTE,
            vec![answered(200, LOGIN_BODY), answered(200, SECOND_LOGIN_BODY)],
        )],
    );
    assert_eq!(
        manager.request_token().await.unwrap().expose_secret(),
        "hvs.first"
    );
    assert_eq!(
        manager.request_token().await.unwrap().expose_secret(),
        "hvs.second"
    );
    assert_eq!(
        manager.request_token_blocking().unwrap().expose_secret(),
        "hvs.first"
    );
    assert_eq!(transport.recorded().len(), 3);
}

#[tokio::test]
async fn token_mode_makes_no_http_call() {
    let (manager, transport) = scripted(&[("HCP_VAULT_TOKEN", "token-1")], &[]);
    assert_eq!(
        manager.request_token().await.unwrap().expose_secret(),
        "token-1"
    );
    assert_eq!(
        manager.request_token_blocking().unwrap().expose_secret(),
        "token-1"
    );
    assert!(transport.recorded().is_empty());
}

#[rstest]
#[case::transport_failure(Reply::Failure, LoginMethod::AppRole)]
#[case::rejected_status(answered(403, "permission denied"), LoginMethod::AppRole)]
#[case::redirect_status(answered(307, ""), LoginMethod::AppRole)]
#[case::invalid_json(answered(200, "not a login response"), LoginMethod::AppRole)]
#[case::missing_token(
    answered(200, "{\"auth\":{\"lease_duration\":60}}"),
    LoginMethod::AppRole
)]
#[case::missing_lease(
    answered(200, "{\"auth\":{\"client_token\":\"hvs.leak\"}}"),
    LoginMethod::AppRole
)]
#[case::negative_lease(
    answered(
        200,
        "{\"auth\":{\"client_token\":\"hvs.leak\",\"lease_duration\":-1}}"
    ),
    LoginMethod::AppRole
)]
#[case::fractional_lease(
    answered(
        200,
        "{\"auth\":{\"client_token\":\"hvs.leak\",\"lease_duration\":1.5}}"
    ),
    LoginMethod::AppRole
)]
#[tokio::test]
async fn approle_login_failures_name_the_method(#[case] reply: Reply, #[case] method: LoginMethod) {
    let (manager, _transport) = scripted(APPROLE, &[(APPROLE_ROUTE, vec![reply])]);
    let error = manager.request_token().await.unwrap_err();
    assert_method(&error, method);
    assert_no_login_secret(&error);
}

#[rstest]
#[case::transport_failure(Reply::Failure, LoginMethod::TlsCert)]
#[case::rejected_status(answered(403, "permission denied"), LoginMethod::TlsCert)]
#[case::redirect_status(answered(307, ""), LoginMethod::TlsCert)]
#[case::invalid_json(answered(200, "not a login response"), LoginMethod::TlsCert)]
#[tokio::test]
async fn tls_login_failures_name_the_method(#[case] reply: Reply, #[case] method: LoginMethod) {
    let (manager, _transport) = scripted(TLS, &[(TLS_ROUTE, vec![reply])]);
    let error = manager.request_token().await.unwrap_err();
    assert_method(&error, method);
    assert_no_login_secret(&error);
}

#[tokio::test]
async fn an_oversized_login_response_is_refused() {
    let oversized = "a".repeat(MAX_RESPONSE_BYTES + 1);
    let (manager, _transport) =
        scripted(APPROLE, &[(APPROLE_ROUTE, vec![answered(200, &oversized)])]);
    let error = manager.request_token().await.unwrap_err();
    assert!(matches!(
        error,
        VaultError::ResponseTooLarge {
            method: LoginMethod::AppRole,
            limit: MAX_RESPONSE_BYTES
        }
    ));
    assert_no_login_secret(&error);
}

#[test]
fn the_approle_request_debug_hides_the_secrets() {
    let (manager, _transport) = scripted(APPROLE, &[]);
    let role_id = secrecy::SecretString::from("role-hidden");
    let secret_id = secrecy::SecretString::from("secret-hidden");
    let channel = super::LoginChannel::AppRole {
        role_id: &role_id,
        secret_id: &secret_id,
        mount_path: "approle",
    };
    let request = super::login_request(&manager, channel).unwrap();
    let debug = format!("{request:?}");
    assert!(!debug.contains("role-hidden"));
    assert!(!debug.contains("secret-hidden"));
}

#[test]
fn the_tls_request_debug_hides_the_body() {
    let (manager, _transport) = scripted(TLS, &[]);
    let certificate = ClientCertificate {
        certificate_path: "/certs/client.crt".to_owned(),
        key_path: "/certs/client.key".to_owned(),
    };
    let channel = super::LoginChannel::TlsCert {
        client_certificate: &certificate,
        role: Some("role-hidden"),
    };
    let request = super::login_request(&manager, channel).unwrap();
    let debug = format!("{request:?}");
    assert!(!debug.contains("role-hidden"));
}

fn assert_method<Error>(error: &VaultError<Error>, expected: LoginMethod) {
    let named = match error {
        VaultError::Transport { method, .. }
        | VaultError::Status { method, .. }
        | VaultError::ResponseTooLarge { method, .. }
        | VaultError::InvalidLogin { method } => *method,
    };
    assert_eq!(named, expected);
}

fn assert_no_login_secret<Error>(error: &VaultError<Error>)
where
    Error: Debug,
{
    let text = format!("{error} {error:?}");
    assert!(!text.contains("hvs.leak"));
    assert!(!text.contains("permission denied"));
}

#[tokio::test]
async fn the_approle_login_logs_each_cache_state() {
    let (logs, _guard) = capture_logs();
    let (manager, _transport) =
        scripted(APPROLE, &[(APPROLE_ROUTE, vec![answered(200, LOGIN_BODY)])]);
    manager.request_token().await.unwrap();
    manager.request_token().await.unwrap();
    let text = logs.text();
    assert!(text.contains("Using AppRole auth for Hashicorp Vault"));
    assert!(text.contains("Successfully obtained Vault token via AppRole auth"));
    assert!(text.contains("Using cached Vault token from AppRole auth"));
    assert!(!text.contains("hvs.first"));
}

#[tokio::test]
async fn the_tls_login_logs_the_channel() {
    let (logs, _guard) = capture_logs();
    let (manager, _transport) = scripted(TLS, &[(TLS_ROUTE, vec![answered(200, LOGIN_BODY)])]);
    manager.request_token().await.unwrap();
    let text = logs.text();
    assert!(text.contains("Using TLS cert auth for Hashicorp Vault"));
    assert!(text.contains("Successfully obtained Vault token via TLS cert auth."));
    assert!(!text.contains("hvs.first"));
}

#[test]
fn the_error_text_names_the_rejected_login_method() {
    let error: VaultError<Refused> = VaultError::Status {
        method: LoginMethod::AppRole,
        status: 403,
    };
    assert!(
        error
            .to_string()
            .starts_with("Could not authenticate to Vault via AppRole")
    );
}
