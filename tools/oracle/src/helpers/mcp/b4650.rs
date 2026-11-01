mod tests;

use betterlite_mcp::credential_contracts::auth_config::AuthConfig;
use betterlite_mcp::credential_contracts::auth_config::aws::{
    AwsCredentialSource, AwsSigV4Config, DEFAULT_AWS_SIGV4_SERVICE,
};
use betterlite_mcp::credential_contracts::auth_config::id_jag::{
    ClientAuth, ClientSecretAuth, ID_JAG_SUBJECT_TOKEN_TYPE, IdJagConfig,
};
use betterlite_mcp::credential_contracts::auth_config::key_source::{
    ApiKeyConfig, ApiKeySource, SharedKey,
};
use betterlite_mcp::credential_contracts::auth_config::oauth::{
    AuthorizationCodeConfig, ClientCredentialsConfig, TokenExchangeConfig,
};
use betterlite_mcp::credential_contracts::auth_config::simple::{NoneConfig, PassthroughConfig};
use betterlite_mcp::credential_contracts::auth_spec::parse_auth_spec_kind;
use betterlite_mcp::credential_contracts::byok_cache::byok_credential_cache_key;
use betterlite_mcp::credential_contracts::errors::{CredError, CredResult};
use betterlite_mcp::credential_contracts::header_carrier::HeaderCarrier;
use betterlite_mcp::credential_contracts::token_endpoint_auth::{
    TokenEndpointAuthConfigError, TokenEndpointAuthMethod, build_token_client_auth,
    normalize_token_auth_method,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ResultUnion {
        ok: bool,
        value: String,
    },
    AuthSpecKind {
        raw: String,
    },
    AuthConfigVariants {
        kind: String,
    },
    TokenAuthNormalize {
        value: String,
    },
    TokenAuthBuild {
        #[serde(default)]
        auth_method: Option<String>,
        #[serde(default)]
        client_id: Option<String>,
        #[serde(default)]
        client_secret: Option<String>,
    },
    ByokCacheKey {
        user_id: String,
        server_id: String,
    },
}

#[derive(Debug, Error)]
enum Failure {
    #[error("unknown kind: '{0}'")]
    UnknownKind(String),
    #[error(transparent)]
    TokenAuth(#[from] TokenEndpointAuthConfigError),
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        let class = match &failure {
            Failure::UnknownKind(_) => "ValueError",
            Failure::TokenAuth(_) => "TokenEndpointAuthConfigError",
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}

fn build_auth_config(kind: &str) -> Result<AuthConfig, Failure> {
    match kind {
        "authorization_code" => Ok(AuthConfig::AuthorizationCode(
            AuthorizationCodeConfig::default(),
        )),
        "client_credentials" => Ok(AuthConfig::ClientCredentials(
            ClientCredentialsConfig::default(),
        )),
        "token_exchange" => Ok(AuthConfig::TokenExchange(TokenExchangeConfig::default())),
        "id_jag" => Ok(AuthConfig::IdJag(IdJagConfig {
            header: HeaderCarrier::default(),
            org_token_endpoint: "https://org.example.test/token".to_owned(),
            resource_token_endpoint: "https://resource.example.test/token".to_owned(),
            client_id: "gateway-client".to_owned(),
            client_auth: ClientAuth::ClientSecret(ClientSecretAuth {
                client_secret: SecretString::from("secret".to_owned()),
            }),
            subject_token_type: ID_JAG_SUBJECT_TOKEN_TYPE.to_owned(),
            audience: None,
            resource: None,
            scopes: Vec::new(),
        })),
        "api_key" => Ok(AuthConfig::ApiKey(ApiKeyConfig {
            header: HeaderCarrier::default(),
            key_source: ApiKeySource::Shared(SharedKey {
                value: SecretString::from("key".to_owned()),
            }),
        })),
        "passthrough" => Ok(AuthConfig::Passthrough(PassthroughConfig)),
        "none" => Ok(AuthConfig::None(NoneConfig)),
        "aws_sigv4" => Ok(AuthConfig::AwsSigv4(AwsSigV4Config {
            region: "us-east-1".to_owned(),
            service: DEFAULT_AWS_SIGV4_SERVICE.to_owned(),
            credentials: AwsCredentialSource::default(),
        })),
        other => Err(Failure::UnknownKind(other.to_owned())),
    }
}

fn result_union(ok: bool, value: &str) -> serde_json::Value {
    let result: CredResult<String> = if ok {
        Ok(value.to_owned())
    } else {
        Err(CredError::Misconfigured(value.to_owned()))
    };
    json!({
        "is_ok": result.is_ok(),
        "is_error": result.is_err(),
        "ok": result.as_ref().ok(),
        "error_summary": result.as_ref().err().map(CredError::summary),
    })
}

fn auth_spec_kind(raw: &str) -> serde_json::Value {
    match parse_auth_spec_kind(raw) {
        Ok(kind) => json!({ "ok": true, "value": kind.to_string() }),
        Err(error) => json!({ "ok": false, "error_summary": error.summary() }),
    }
}

fn token_auth_build(
    auth_method: Option<&str>,
    client_id: Option<&str>,
    client_secret: Option<&str>,
) -> Result<serde_json::Value, Failure> {
    let method = auth_method.and_then(normalize_token_auth_method);
    let built = build_token_client_auth(method, client_id, client_secret)?;
    Ok(json!({ "headers": built.headers, "body": built.body }))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ResultUnion { ok, value } => Ok(result_union(*ok, value)),
            Input::AuthSpecKind { raw } => Ok(auth_spec_kind(raw)),
            Input::AuthConfigVariants { kind } => {
                let config = build_auth_config(kind)?;
                Ok(json!({ "kind": config.kind().to_string() }))
            }
            Input::TokenAuthNormalize { value } => Ok(json!({
                "value": normalize_token_auth_method(value).map(TokenEndpointAuthMethod::as_str),
            })),
            Input::TokenAuthBuild {
                auth_method,
                client_id,
                client_secret,
            } => Ok(token_auth_build(
                auth_method.as_deref(),
                client_id.as_deref(),
                client_secret.as_deref(),
            )?),
            Input::ByokCacheKey { user_id, server_id } => {
                Ok(json!({ "key": byok_credential_cache_key(user_id, server_id) }))
            }
        }
    }
}
