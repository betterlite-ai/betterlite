use std::collections::BTreeMap;

use betterlite_config::sections::azure_credentials::DEFAULT_SCOPE;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use crate::credentials::auth_headers::{AuthHeaderRequest, get_azure_ai_auth_headers};
use crate::credentials::error::CredentialsError;
use crate::credentials::host::ApiKeyHeader;

use super::builders::{auth_request, environment, headers, object};
use super::support::FixtureSources;

pub(super) fn explicit_key_before_entra() {
    explicit_key_headers();
    key_wins_over_entra();
    entra_fallback();
}

fn explicit_key_headers() {
    let environment = environment(&[]);
    let sources = FixtureSources::new();
    let key = SecretString::from("explicit-key".to_owned());
    let bearer = header_result(
        Some(&key),
        None,
        &environment,
        ApiKeyHeader::Authorization,
        &sources,
    );
    assert_eq!(bearer, headers(&[("Authorization", "Bearer explicit-key")]));
    let api_key = header_result(
        Some(&key),
        None,
        &environment,
        ApiKeyHeader::ApiKey,
        &sources,
    );
    assert_eq!(api_key, headers(&[("api-key", "explicit-key")]));
    let empty = SecretString::from(String::new());
    let empty_key = header_result(
        Some(&empty),
        None,
        &environment,
        ApiKeyHeader::Authorization,
        &sources,
    );
    assert_eq!(empty_key, headers(&[("Authorization", "Bearer ")]));
    assert!(sources.calls().is_empty());
}

fn key_wins_over_entra() {
    let environment = environment(&[]);
    let sources = FixtureSources::new();
    let params = entra_params();
    let key = SecretString::from("explicit-key".to_owned());
    let result = header_result(
        Some(&key),
        Some(&params),
        &environment,
        ApiKeyHeader::Authorization,
        &sources,
    );
    assert_eq!(result, headers(&[("Authorization", "Bearer explicit-key")]));
    assert!(sources.calls().is_empty());
}

fn entra_fallback() {
    grant_fallback();
    static_fallback();
}

fn grant_fallback() {
    let environment = environment(&[]);
    let sources = FixtureSources::new();
    let params = entra_params();
    let result = header_result(
        None,
        Some(&params),
        &environment,
        ApiKeyHeader::Authorization,
        &sources,
    );
    assert_eq!(
        result,
        headers(&[("Authorization", "Bearer client-secret-token")])
    );
    assert_eq!(
        sources.calls(),
        vec![format!(
            "client_secret tenant=tenant client=client secret=secret scope={DEFAULT_SCOPE}"
        )]
    );
}

fn static_fallback() {
    let empty_env = environment(&[]);
    let static_token = object([("azure_ad_token", Json::Text("static-token".to_owned()))]);
    let sources = FixtureSources::new();
    let direct = header_result(
        None,
        Some(&static_token),
        &empty_env,
        ApiKeyHeader::Authorization,
        &sources,
    );
    assert_eq!(direct, headers(&[("Authorization", "Bearer static-token")]));
    assert!(sources.calls().is_empty());
    let with_env = environment(&[("AZURE_AD_TOKEN", "env-token")]);
    let env_sources = FixtureSources::new();
    let empty = object([]);
    let from_env = header_result(
        None,
        Some(&empty),
        &with_env,
        ApiKeyHeader::Authorization,
        &env_sources,
    );
    assert_eq!(from_env, headers(&[("Authorization", "Bearer env-token")]));
    assert!(env_sources.calls().is_empty());
}

fn header_result(
    api_key: Option<&SecretString>,
    params: Option<&Object>,
    environment: &BTreeMap<String, String>,
    header: ApiKeyHeader,
    sources: &FixtureSources,
) -> BTreeMap<String, String> {
    get_azure_ai_auth_headers(&auth_request(api_key, params, environment, header, sources)).unwrap()
}

fn entra_params() -> Object {
    object([
        ("tenant_id", Json::Text("tenant".to_owned())),
        ("client_id", Json::Text("client".to_owned())),
        ("client_secret", Json::Text("secret".to_owned())),
    ])
}

pub(super) fn missing_credentials() {
    let environment = environment(&[]);
    let sources = FixtureSources::new();
    let error = get_azure_ai_auth_headers(&auth_request(
        None,
        None,
        &environment,
        ApiKeyHeader::Authorization,
        &sources,
    ))
    .unwrap_err();
    assert!(matches!(error, CredentialsError::MissingCredentials { .. }));
    let message = error.to_string();
    for fragment in [
        "Missing Azure AI credentials",
        "`api_key` or AZURE_AI_API_KEY",
        "`tenant_id` + `client_id` + `client_secret`",
        "`azure_ad_token`",
        "an OIDC token",
        "managed identity",
        "`litellm.enable_azure_ad_token_refresh = True`",
    ] {
        assert!(message.contains(fragment), "{fragment}");
    }
    assert!(sources.calls().is_empty());
    partial_credentials();
}

fn partial_credentials() {
    let empty_env = environment(&[]);
    let sources = FixtureSources::new();
    let tenant_only = object([
        ("tenant_id", Json::Text("tenant".to_owned())),
        ("client_id", Json::Text("client".to_owned())),
    ]);
    let missing = request_error(Some(&tenant_only), &empty_env, &sources);
    assert!(matches!(
        missing,
        CredentialsError::MissingCredentials { .. }
    ));
    let empty_token = object([("azure_ad_token", Json::Text(String::new()))]);
    let empty = request_error(Some(&empty_token), &empty_env, &sources);
    assert!(matches!(empty, CredentialsError::MissingCredentials { .. }));
    let empty_variable = environment(&[("AZURE_AD_TOKEN", "")]);
    let from_empty_env = request_error(None, &empty_variable, &sources);
    assert!(matches!(
        from_empty_env,
        CredentialsError::MissingCredentials { .. }
    ));
    let custom = AuthHeaderRequest {
        api_key: None,
        params: None,
        environment: &empty_env,
        api_key_header: ApiKeyHeader::Authorization,
        api_key_env_var: "CUSTOM_AI_KEY",
        refresh_enabled: false,
        sources: &sources,
    };
    let message = get_azure_ai_auth_headers(&custom).unwrap_err().to_string();
    assert!(message.contains("`api_key` or CUSTOM_AI_KEY"));
    assert!(sources.calls().is_empty());
}

fn request_error(
    params: Option<&Object>,
    environment: &BTreeMap<String, String>,
    sources: &FixtureSources,
) -> CredentialsError {
    get_azure_ai_auth_headers(&auth_request(
        None,
        params,
        environment,
        ApiKeyHeader::Authorization,
        sources,
    ))
    .unwrap_err()
}
