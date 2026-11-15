use std::collections::BTreeMap;

use betterlite_azure::credentials::source::SourceFailure;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::agent::{get_azure_ai_agent_entra_token, resolve_agent_auth_header};
use crate::credentials::auth_headers::get_azure_ai_auth_headers;
use crate::credentials::entra::{EntraTokenRequest, get_azure_ai_entra_token};
use crate::credentials::error::CredentialsError;
use crate::credentials::host::ApiKeyHeader;
use crate::credentials::params::EntraParams;

use super::builders::{accessors, agent_request, auth_request, environment, headers, object};
use super::support::{FixtureSources, Outcome, secrets};

const SENTINEL: &str = "sentinel-secret-value";

pub(super) fn credential_identity_isolation() {
    header_identities();
    agent_identities();
    agent_header_identity();
    environment_identities();
}

fn header_identities() {
    let environment = environment(&[]);
    let first_key = SecretString::from("key-first".to_owned());
    let second_key = SecretString::from("key-second".to_owned());
    let first = get_azure_ai_auth_headers(&auth_request(
        Some(&first_key),
        None,
        &environment,
        ApiKeyHeader::Authorization,
        &FixtureSources::new(),
    ))
    .unwrap();
    let second = get_azure_ai_auth_headers(&auth_request(
        Some(&second_key),
        None,
        &environment,
        ApiKeyHeader::ApiKey,
        &FixtureSources::new(),
    ))
    .unwrap();
    assert_eq!(first, headers(&[("Authorization", "Bearer key-first")]));
    assert_eq!(second, headers(&[("api-key", "key-second")]));
    assert!(!first.values().any(|value| value.contains("key-second")));
    assert!(!second.values().any(|value| value.contains("key-first")));
}

fn agent_identities() {
    let environment = environment(&[]);
    let secrets = secrets();
    let first_params = identity("tenant-first", "client-first", "secret-first");
    let second_params = identity("tenant-second", "client-second", "secret-second");
    let mut first_sources = FixtureSources::new();
    first_sources.client_secret = Ok(Outcome::Token("token-first".to_owned()));
    let mut second_sources = FixtureSources::new();
    second_sources.client_secret = Ok(Outcome::Token("token-second".to_owned()));
    let first = get_azure_ai_agent_entra_token(&agent_request(
        &first_params,
        &environment,
        &secrets,
        &first_sources,
    ))
    .unwrap();
    let second = get_azure_ai_agent_entra_token(&agent_request(
        &second_params,
        &environment,
        &secrets,
        &second_sources,
    ))
    .unwrap();
    assert_eq!(first.expose_secret(), "token-first");
    assert_eq!(second.expose_secret(), "token-second");
    let first_calls = first_sources.calls();
    let first_call = &first_calls[0];
    assert!(first_call.contains("tenant=tenant-first"), "{first_call}");
    assert!(!first_call.contains("secret-second"), "{first_call}");
    let second_calls = second_sources.calls();
    let second_call = &second_calls[0];
    assert!(
        second_call.contains("client=client-second"),
        "{second_call}"
    );
    assert!(!second_call.contains("tenant-first"), "{second_call}");
}

fn agent_header_identity() {
    let environment = environment(&[]);
    let secrets = secrets();
    let params = identity("tenant-first", "client-first", "secret-first");
    let mut sources = FixtureSources::new();
    sources.client_secret = Ok(Outcome::Token("token-first".to_owned()));
    let header =
        resolve_agent_auth_header(&agent_request(&params, &environment, &secrets, &sources))
            .unwrap();
    assert_eq!(header, headers(&[("Authorization", "Bearer token-first")]));
}

fn identity(tenant: &str, client: &str, secret: &str) -> Object {
    object([
        ("tenant_id", Json::Text(tenant.to_owned())),
        ("client_id", Json::Text(client.to_owned())),
        ("client_secret", Json::Text(secret.to_owned())),
    ])
}

fn environment_identities() {
    let first = env_headers(&[("AZURE_AD_TOKEN", "env-token-first")]);
    let second = env_headers(&[("AZURE_AD_TOKEN", "env-token-second")]);
    assert_eq!(
        first,
        headers(&[("Authorization", "Bearer env-token-first")])
    );
    assert_eq!(
        second,
        headers(&[("Authorization", "Bearer env-token-second")])
    );
    assert_ne!(first, second);
}

fn env_headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    let environment = environment(pairs);
    get_azure_ai_auth_headers(&auth_request(
        None,
        None,
        &environment,
        ApiKeyHeader::Authorization,
        &FixtureSources::new(),
    ))
    .unwrap()
}

pub(super) fn secret_safe_failure() {
    unresolved_hides_secret();
    provider_failure_hides_secret();
    validation_error_hides_secret();
    parameter_debug_redacts_secret();
}

fn assert_hidden(error: &CredentialsError) {
    assert!(!error.to_string().contains(SENTINEL));
    assert!(!format!("{error:?}").contains(SENTINEL));
}

fn unresolved_hides_secret() {
    let environment = environment(&[]);
    let secrets = secrets();
    let sources = FixtureSources::new();
    let params = object([("client_secret", Json::Text(SENTINEL.to_owned()))]);
    let error =
        get_azure_ai_agent_entra_token(&agent_request(&params, &environment, &secrets, &sources))
            .unwrap_err();
    assert_hidden(&error);
}

fn provider_failure_hides_secret() {
    let environment = environment(&[]);
    let secrets = secrets();
    let mut sources = FixtureSources::new();
    sources.client_secret = Err(SourceFailure {
        class: "ValueError".to_owned(),
        status: None,
        message: "provider refused".to_owned(),
    });
    let params = identity("tenant", "client", SENTINEL);
    let error =
        get_azure_ai_agent_entra_token(&agent_request(&params, &environment, &secrets, &sources))
            .unwrap_err();
    assert!(error.to_string().contains("provider refused"));
    assert_hidden(&error);
}

fn validation_error_hides_secret() {
    let environment = environment(&[]);
    let sources = FixtureSources::new();
    let params = object([
        ("tenant_id", Json::Bool(true)),
        ("client_secret", Json::Text(SENTINEL.to_owned())),
    ]);
    let error = get_azure_ai_entra_token(&EntraTokenRequest {
        params: Some(&params),
        environment: &environment,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap_err();
    assert!(matches!(
        error,
        CredentialsError::InvalidParam { field: "tenant_id" }
    ));
    assert_hidden(&error);
}

fn parameter_debug_redacts_secret() {
    let params = identity("tenant", "client", SENTINEL);
    let resolved = EntraParams::from_mapping(Some(&params)).unwrap();
    assert!(!format!("{resolved:?}").contains(SENTINEL));
    let secret = SecretString::from(SENTINEL.to_owned());
    let view = accessors(None, None, None, Some(&secret));
    assert!(!format!("{view:?}").contains(SENTINEL));
}
