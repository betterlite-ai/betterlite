#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::microsoft_sso::{
    AUTHORIZATION_ENDPOINT_ENV, CLIENT_SECRET_ENV, Settings, TENANT_ENV, TOKEN_ENDPOINT_ENV,
    USERINFO_ENDPOINT_ENV,
};
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use secrecy::ExposeSecret as _;
use tracing::subscriber::with_default;

use crate::oauth_client::authorization::{AuthorizationRequest, authorization_url};
use crate::oauth_client::microsoft::{
    DEFAULT_TENANT, MICROSOFT_SCOPES, MicrosoftClientError, MicrosoftSsoClient,
    microsoft_default_discovery, microsoft_sso_client,
};

const CLIENT_ID: &str = "microsoft-client-id";
const REDIRECT_URL: &str = "https://proxy.example.test/sso/callback";
const COMMON_AUTHORIZATION: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const COMMON_TOKEN: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const USERINFO: &str = "https://graph.microsoft.com/v1.0/me";

fn settings(pairs: &[(&str, &str)]) -> Settings {
    let environment = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Settings::resolve_environment(&environment)
}

fn configured_client(pairs: &[(&str, &str)]) -> MicrosoftSsoClient {
    microsoft_sso_client(&settings(pairs), CLIENT_ID, REDIRECT_URL).unwrap()
}

#[test]
fn an_absent_tenant_uses_common() {
    let client = configured_client(&[(CLIENT_SECRET_ENV, "microsoft-secret")]);
    assert_eq!(client.tenant, "common");
    assert_eq!(
        client.discovery.authorization_endpoint,
        COMMON_AUTHORIZATION
    );
    assert_eq!(client.discovery.token_endpoint, COMMON_TOKEN);
    assert_eq!(client.discovery.userinfo_endpoint, USERINFO);
}

#[test]
fn an_empty_tenant_uses_common() {
    let client = configured_client(&[(CLIENT_SECRET_ENV, "microsoft-secret"), (TENANT_ENV, "")]);
    assert_eq!(client.tenant, DEFAULT_TENANT);
    assert_eq!(
        client.discovery.authorization_endpoint,
        COMMON_AUTHORIZATION
    );
    assert_eq!(client.discovery.token_endpoint, COMMON_TOKEN);
}

#[test]
fn a_named_tenant_appears_in_both_login_urls() {
    let client = configured_client(&[
        (CLIENT_SECRET_ENV, "microsoft-secret"),
        (TENANT_ENV, "contoso.example.test"),
    ]);
    assert_eq!(client.tenant, "contoso.example.test");
    assert_eq!(
        client.discovery.authorization_endpoint,
        "https://login.microsoftonline.com/contoso.example.test/oauth2/v2.0/authorize"
    );
    assert_eq!(
        client.discovery.token_endpoint,
        "https://login.microsoftonline.com/contoso.example.test/oauth2/v2.0/token"
    );
    assert_eq!(client.discovery.userinfo_endpoint, USERINFO);
}

#[test]
fn the_tenant_is_inserted_as_plain_text() {
    let document = microsoft_default_discovery("tenant name+one");
    assert_eq!(
        document.authorization_endpoint,
        "https://login.microsoftonline.com/tenant name+one/oauth2/v2.0/authorize"
    );
    assert_eq!(
        document.token_endpoint,
        "https://login.microsoftonline.com/tenant name+one/oauth2/v2.0/token"
    );
}

#[test]
fn an_authorization_override_replaces_only_the_authorization_endpoint() {
    let client = configured_client(&[
        (CLIENT_SECRET_ENV, "microsoft-secret"),
        (
            AUTHORIZATION_ENDPOINT_ENV,
            "https://login.example.test/authorize",
        ),
    ]);
    assert_eq!(
        client.discovery.authorization_endpoint,
        "https://login.example.test/authorize"
    );
    assert_eq!(client.discovery.token_endpoint, COMMON_TOKEN);
    assert_eq!(client.discovery.userinfo_endpoint, USERINFO);
}

#[test]
fn a_token_override_replaces_only_the_token_endpoint() {
    let client = configured_client(&[
        (CLIENT_SECRET_ENV, "microsoft-secret"),
        (TOKEN_ENDPOINT_ENV, "https://login.example.test/token"),
    ]);
    assert_eq!(
        client.discovery.token_endpoint,
        "https://login.example.test/token"
    );
    assert_eq!(
        client.discovery.authorization_endpoint,
        COMMON_AUTHORIZATION
    );
    assert_eq!(client.discovery.userinfo_endpoint, USERINFO);
}

#[test]
fn a_userinfo_override_replaces_only_the_userinfo_endpoint() {
    let client = configured_client(&[
        (CLIENT_SECRET_ENV, "microsoft-secret"),
        (USERINFO_ENDPOINT_ENV, "https://graph.example.test/me"),
    ]);
    assert_eq!(
        client.discovery.userinfo_endpoint,
        "https://graph.example.test/me"
    );
    assert_eq!(
        client.discovery.authorization_endpoint,
        COMMON_AUTHORIZATION
    );
    assert_eq!(client.discovery.token_endpoint, COMMON_TOKEN);
}

#[test]
fn empty_overrides_fall_back_to_the_defaults() {
    let client = configured_client(&[
        (CLIENT_SECRET_ENV, "microsoft-secret"),
        (AUTHORIZATION_ENDPOINT_ENV, ""),
        (TOKEN_ENDPOINT_ENV, ""),
        (USERINFO_ENDPOINT_ENV, ""),
    ]);
    assert_eq!(
        client.discovery,
        microsoft_default_discovery(DEFAULT_TENANT)
    );
}

#[test]
fn the_client_carries_the_scopes_redirect_and_insecure_flag() {
    let client = configured_client(&[(CLIENT_SECRET_ENV, "microsoft-secret")]);
    assert_eq!(client.oauth.client_id, CLIENT_ID);
    assert_eq!(client.oauth.redirect_uri.as_deref(), Some(REDIRECT_URL));
    assert_eq!(client.oauth.scopes, MICROSOFT_SCOPES);
    assert!(client.oauth.allow_insecure_http);
    assert!(client.oauth.additional_headers.is_empty());
}

#[test]
fn an_empty_secret_is_accepted() {
    let client = configured_client(&[(CLIENT_SECRET_ENV, "")]);
    assert_eq!(client.oauth.client_secret.expose_secret(), "");
}

#[test]
fn a_missing_secret_is_a_500_auth_error_naming_the_variable() {
    let error = microsoft_sso_client(&settings(&[]), CLIENT_ID, REDIRECT_URL)
        .err()
        .expect("the missing secret is refused");
    assert!(matches!(&error, MicrosoftClientError::MissingClientSecret));
    let response = error.response();
    assert_eq!(response.status, 500);
    assert_eq!(response.error_type, ProxyErrorTypes::AuthError);
    assert_eq!(response.param, Some("MICROSOFT_CLIENT_SECRET"));
    assert_eq!(response.code, "500");
}

#[test]
fn the_login_url_comes_from_the_client_and_its_discovery() {
    let client = configured_client(&[(CLIENT_SECRET_ENV, "microsoft-secret")]);
    let request = AuthorizationRequest {
        endpoint: client.discovery.authorization_endpoint.clone(),
        state: None,
        redirect_uri: None,
        extra_params: Vec::new(),
    };
    let url = authorization_url(&client.oauth, &request).unwrap();
    assert_eq!(
        url.as_str(),
        "https://login.microsoftonline.com/common/oauth2/v2.0/authorize?response_type=code&client_id=microsoft-client-id&redirect_uri=https%3A%2F%2Fproxy.example.test%2Fsso%2Fcallback&scope=openid+User.Read+email"
    );
}

#[test]
fn a_non_empty_override_logs_the_effective_endpoints_at_debug() {
    let settings = settings(&[
        (CLIENT_SECRET_ENV, "microsoft-secret"),
        (TOKEN_ENDPOINT_ENV, "https://login.example.test/token"),
    ]);
    let (client, logs) =
        capture(|| microsoft_sso_client(&settings, CLIENT_ID, REDIRECT_URL).unwrap());
    assert_eq!(
        client.discovery.token_endpoint,
        "https://login.example.test/token"
    );
    assert!(logs.contains(
        "Using custom Microsoft SSO endpoints - authorization: https://login.microsoftonline.com/common/oauth2/v2.0/authorize, token: https://login.example.test/token, userinfo: https://graph.microsoft.com/v1.0/me"
    ));
}

#[test]
fn overrides_that_are_absent_or_empty_log_nothing() {
    let settings = settings(&[
        (CLIENT_SECRET_ENV, "microsoft-secret"),
        (TOKEN_ENDPOINT_ENV, ""),
    ]);
    let (_, logs) = capture(|| microsoft_sso_client(&settings, CLIENT_ID, REDIRECT_URL).unwrap());
    assert!(!logs.contains("Using custom Microsoft SSO endpoints"));
}

#[derive(Clone)]
struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture<T, F>(operation: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = LogWriter(Arc::clone(&bytes));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || writer.clone())
        .finish();
    let output = with_default(subscriber, operation);
    let text =
        String::from_utf8(bytes.lock().unwrap_or_else(PoisonError::into_inner).clone()).unwrap();
    (output, text)
}
