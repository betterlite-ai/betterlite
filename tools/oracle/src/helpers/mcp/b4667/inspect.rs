use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use betterlite_mcp::credential_contracts::auth_config::AuthConfig;
use betterlite_mcp::credential_contracts::auth_config::aws::{AwsCredentialSource, AwsSigV4Config};
use betterlite_mcp::credential_contracts::auth_config::id_jag::{ClientAuth, IdJagConfig};
use betterlite_mcp::credential_contracts::auth_config::key_source::{ApiKeyConfig, ApiKeySource};
use betterlite_mcp::credential_contracts::auth_config::oauth::{
    AuthorizationCodeConfig, ClientCredentialsConfig, TokenExchangeConfig, TokenExchangeProfile,
};
use betterlite_mcp::credential_contracts::header_carrier::HeaderCarrier;
use betterlite_mcp::credential_contracts::token_endpoint_auth::TokenEndpointAuthMethod;
use betterlite_mcp::outbound_credentials::adapter;
use betterlite_mcp::server_record::MCPServer;
use betterlite_mcp::wire_compat::errors::HttpException;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

pub(super) fn spec_answer(server: &MCPServer) -> HelperResult {
    let Some(spec) = adapter::to_server_spec(server).or_value_error()? else {
        return Ok(Value::Null);
    };
    Ok(json!({
        "server_id": spec.server_id,
        "resource": spec.resource,
        "config": config_answer(&spec.config),
    }))
}

pub(super) fn challenge_answer(exception: &HttpException) -> Value {
    let headers: BTreeMap<String, String> = exception
        .headers
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value.clone()))
        .collect();
    json!({"status_code": exception.status_code, "headers": headers})
}

fn config_answer(config: &AuthConfig) -> Value {
    match config {
        AuthConfig::AuthorizationCode(fields) => authorization_code_answer(fields),
        AuthConfig::ClientCredentials(fields) => client_credentials_answer(fields),
        AuthConfig::TokenExchange(fields) => token_exchange_answer(fields),
        AuthConfig::IdJag(fields) => id_jag_answer(fields),
        AuthConfig::ApiKey(fields) => api_key_answer(fields),
        AuthConfig::Passthrough(_fields) => plain_answer("passthrough"),
        AuthConfig::None(_fields) => plain_answer("none"),
        AuthConfig::AwsSigv4(fields) => aws_answer(fields),
    }
}

fn plain_answer(kind: &str) -> Value {
    json!({"kind": kind, "fields": {}})
}

fn with_header(kind: &str, header: &HeaderCarrier, fields: Value) -> Value {
    let (name, prefix) = header_parts(header);
    json!({"kind": kind, "header": {"name": name, "prefix": prefix}, "fields": fields})
}

fn api_key_answer(config: &ApiKeyConfig) -> Value {
    let source = match &config.key_source {
        ApiKeySource::Shared(shared) => {
            json!({"kind": "shared", "value": shared.value.expose_secret()})
        }
        ApiKeySource::Byok => json!({"kind": "byok"}),
    };
    with_header("api_key", &config.header, json!({"key_source": source}))
}

fn authorization_code_answer(config: &AuthorizationCodeConfig) -> Value {
    with_header(
        "authorization_code",
        &config.header,
        json!({
            "scopes": config.scopes,
            "client_id": config.client_id,
            "client_secret": secret_text(config.client_secret.as_ref()),
            "authorization_url": config.authorization_url,
            "token_url": config.token_url,
        }),
    )
}

fn client_credentials_answer(config: &ClientCredentialsConfig) -> Value {
    with_header(
        "client_credentials",
        &config.header,
        json!({
            "client_id": config.client_id,
            "client_secret": secret_text(config.client_secret.as_ref()),
            "token_url": config.token_url,
            "scopes": config.scopes,
            "audience": config.audience,
            "upstream_resource": config.upstream_resource,
            "token_endpoint_auth_method": auth_method(config.token_endpoint_auth_method),
        }),
    )
}

fn token_exchange_answer(config: &TokenExchangeConfig) -> Value {
    with_header(
        "token_exchange",
        &config.header,
        json!({
            "profile": profile(config.profile),
            "subject_token_type": config.subject_token_type,
            "token_exchange_endpoint": config.token_exchange_endpoint,
            "audience": config.audience,
            "client_id": config.client_id,
            "client_secret": secret_text(config.client_secret.as_ref()),
            "token_endpoint_auth_method": auth_method(config.token_endpoint_auth_method),
            "scopes": config.scopes,
        }),
    )
}

fn id_jag_answer(config: &IdJagConfig) -> Value {
    with_header(
        "id_jag",
        &config.header,
        json!({
            "org_token_endpoint": config.org_token_endpoint,
            "resource_token_endpoint": config.resource_token_endpoint,
            "client_id": config.client_id,
            "client_auth": client_auth_answer(&config.client_auth),
            "subject_token_type": config.subject_token_type,
            "audience": config.audience,
            "resource": config.resource,
            "scopes": config.scopes,
        }),
    )
}

pub(super) fn client_auth_answer(auth: &ClientAuth) -> Value {
    match auth {
        ClientAuth::PrivateKeyJwt(jwt) => json!({
            "source": "private_key_jwt",
            "private_key": jwt.private_key.expose_secret(),
            "key_id": jwt.key_id,
            "signing_alg": jwt.signing_alg,
        }),
        ClientAuth::ClientSecret(secret) => json!({
            "source": "client_secret",
            "client_secret": secret.client_secret.expose_secret(),
        }),
    }
}

fn aws_answer(config: &AwsSigV4Config) -> Value {
    let credentials = match &config.credentials {
        AwsCredentialSource::StaticKeys(keys) => json!({
            "kind": "static_keys",
            "access_key_id": keys.access_key_id,
            "secret_access_key": keys.secret_access_key.expose_secret(),
            "session_token": secret_text(keys.session_token.as_ref()),
        }),
        AwsCredentialSource::AssumeRole(role) => json!({
            "kind": "assume_role",
            "role_arn": role.role_arn,
            "session_name": role.session_name,
            "external_id": role.external_id,
        }),
        AwsCredentialSource::Ambient => json!({"kind": "ambient"}),
    };
    json!({
        "kind": "aws_sigv4",
        "fields": {"region": config.region, "service": config.service, "credentials": credentials},
    })
}

fn header_parts(carrier: &HeaderCarrier) -> (String, String) {
    let (name, formatted) = carrier.header("probe");
    let prefix = formatted
        .strip_suffix(" probe")
        .unwrap_or_default()
        .to_owned();
    (name, prefix)
}

fn secret_text(value: Option<&SecretString>) -> Option<String> {
    value.map(|secret| secret.expose_secret().to_owned())
}

fn auth_method(method: Option<TokenEndpointAuthMethod>) -> Option<&'static str> {
    method.map(TokenEndpointAuthMethod::as_str)
}

const fn profile(profile: TokenExchangeProfile) -> &'static str {
    match profile {
        TokenExchangeProfile::Rfc8693 => "rfc8693",
        TokenExchangeProfile::EntraObo => "entra_obo",
    }
}
