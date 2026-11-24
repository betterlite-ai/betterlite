use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use betterlite_mcp::credential_contracts::auth_config::AuthConfig;
use betterlite_mcp::credential_contracts::auth_config::aws::{AwsCredentialSource, AwsSigV4Config};
use betterlite_mcp::credential_contracts::auth_config::id_jag::ClientAuth;
use betterlite_mcp::credential_contracts::auth_config::key_source::ApiKeySource;
use betterlite_mcp::credential_contracts::auth_config::oauth::TokenExchangeProfile;
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

pub(super) fn resource_path(server: &MCPServer, root_path: &str, scalar_root_path: &str) -> Value {
    let path =
        adapter::challenge::oauth_protected_resource_path(root_path, scalar_root_path, server);
    Value::String(path)
}

pub(super) fn user_challenge(server: &MCPServer, root_path: &str, scalar_root_path: &str) -> Value {
    let exception =
        adapter::challenge::raise_user_oauth_challenge(server, root_path, scalar_root_path);
    challenge_answer(&exception)
}

pub(super) fn exchange_challenge(
    server: &MCPServer,
    root_path: &str,
    scalar_root_path: &str,
    claims: Option<&str>,
) -> Value {
    let exception = adapter::challenge::raise_token_exchange_challenge(
        server,
        root_path,
        scalar_root_path,
        claims,
    );
    challenge_answer(&exception)
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
        AuthConfig::AuthorizationCode(fields) => with_header(
            "authorization_code",
            &fields.header,
            &json!({"scopes": fields.scopes, "client_id": fields.client_id, "client_secret": fields.client_secret.as_ref().map(|secret| secret.expose_secret().to_owned()), "authorization_url": fields.authorization_url, "token_url": fields.token_url}),
        ),
        AuthConfig::ClientCredentials(fields) => with_header(
            "client_credentials",
            &fields.header,
            &json!({"client_id": fields.client_id, "client_secret": fields.client_secret.as_ref().map(|secret| secret.expose_secret().to_owned()), "token_url": fields.token_url, "scopes": fields.scopes, "audience": fields.audience, "upstream_resource": fields.upstream_resource, "token_endpoint_auth_method": fields.token_endpoint_auth_method.map(TokenEndpointAuthMethod::as_str)}),
        ),
        AuthConfig::TokenExchange(fields) => with_header(
            "token_exchange",
            &fields.header,
            &json!({"profile": profile(fields.profile), "subject_token_type": fields.subject_token_type, "token_exchange_endpoint": fields.token_exchange_endpoint, "audience": fields.audience, "client_id": fields.client_id, "client_secret": fields.client_secret.as_ref().map(|secret| secret.expose_secret().to_owned()), "token_endpoint_auth_method": fields.token_endpoint_auth_method.map(TokenEndpointAuthMethod::as_str), "scopes": fields.scopes}),
        ),
        AuthConfig::IdJag(fields) => with_header(
            "id_jag",
            &fields.header,
            &json!({"org_token_endpoint": fields.org_token_endpoint, "resource_token_endpoint": fields.resource_token_endpoint, "client_id": fields.client_id, "client_auth": client_auth_answer(&fields.client_auth), "subject_token_type": fields.subject_token_type, "audience": fields.audience, "resource": fields.resource, "scopes": fields.scopes}),
        ),
        AuthConfig::ApiKey(fields) => with_header(
            "api_key",
            &fields.header,
            &json!({"key_source": key_source_answer(&fields.key_source)}),
        ),
        AuthConfig::Passthrough(_fields) => json!({"kind": "passthrough", "fields": {}}),
        AuthConfig::None(_fields) => json!({"kind": "none", "fields": {}}),
        AuthConfig::AwsSigv4(fields) => aws_sigv4_answer(fields),
    }
}

fn key_source_answer(source: &ApiKeySource) -> Value {
    match source {
        ApiKeySource::Shared(shared) => {
            json!({"kind": "shared", "value": shared.value.expose_secret()})
        }
        ApiKeySource::Byok => json!({"kind": "byok"}),
    }
}

fn aws_sigv4_answer(fields: &AwsSigV4Config) -> Value {
    json!({
        "kind": "aws_sigv4",
        "fields": {
            "region": fields.region,
            "service": fields.service,
            "credentials": match &fields.credentials {
                AwsCredentialSource::StaticKeys(keys) => json!({
                    "kind": "static_keys",
                    "access_key_id": keys.access_key_id,
                    "secret_access_key": keys.secret_access_key.expose_secret(),
                    "session_token": keys.session_token.as_ref().map(|secret| secret.expose_secret().to_owned()),
                }),
                AwsCredentialSource::AssumeRole(role) => json!({
                    "kind": "assume_role",
                    "role_arn": role.role_arn,
                    "session_name": role.session_name,
                    "external_id": role.external_id,
                }),
                AwsCredentialSource::Ambient => json!({"kind": "ambient"}),
            },
        },
    })
}

fn with_header(kind: &str, header: &HeaderCarrier, fields: &Value) -> Value {
    let (name, prefix) = header_parts(header);
    json!({"kind": kind, "header": {"name": name, "prefix": prefix}, "fields": fields})
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

fn header_parts(carrier: &HeaderCarrier) -> (String, String) {
    let (name, formatted) = carrier.header("probe");
    let prefix = formatted
        .strip_suffix(" probe")
        .unwrap_or_default()
        .to_owned();
    (name, prefix)
}

const fn profile(profile: TokenExchangeProfile) -> &'static str {
    match profile {
        TokenExchangeProfile::Rfc8693 => "rfc8693",
        TokenExchangeProfile::EntraObo => "entra_obo",
    }
}
