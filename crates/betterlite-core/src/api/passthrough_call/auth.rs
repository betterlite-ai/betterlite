use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_azure::passthrough::config::{
    AzurePassthroughConfig, AzurePassthroughEnvironment, validate_environment as azure_validate,
};
use betterlite_azure::passthrough::error::AzurePassthroughError;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_config::sections::gigachat_auth::Settings as GigachatSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_context::resource_config_selection::passthrough::PassthroughConfigTag;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_foundry::passthrough::auth::{
    ValidateEnvironmentRequest, validate_environment as azure_ai_validate,
};
use betterlite_http::gigachat_tokens::authenticator::GigachatAuthenticator;
use betterlite_http::gigachat_tokens::request::GigachatTokenRequest;
use betterlite_http::gigachat_tokens::runtime::SystemRuntime;
use betterlite_http::gigachat_tokens::tokens::GigachatTokens as _;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::python_json::dumps_object;
use betterlite_openai::gigachat_chat::config::GigachatBearer;
use betterlite_openai::gigachat_chat::error::GigaChatError;
use betterlite_openai::gigachat_passthrough::{
    GigachatPassthroughConfig, GigachatPassthroughError, validate_environment as gigachat_validate,
};
use betterlite_openai::nvidia_nim_passthrough::validate_environment as nvidia_nim_validate;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::types::PassthroughSigning;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_passthrough::{WatsonxPassthroughConfig, WatsonxPassthroughError};
use betterlite_prompts::arguments::json::Object as PromptsObject;
use secrecy::{ExposeSecret as _, SecretString};

use super::config::ConfigError;
use super::logging::json_object;

const AUTHORIZATION: &str = "Authorization";
const CONTENT_TYPE: &str = "Content-Type";
const JSON_CONTENT: &str = "application/json";
const ACCESS_TOKEN_PARAM: &str = "gigachat_access_token";
const SCOPE_PARAM: &str = "gigachat_scope";
const AUTH_URL_PARAM: &str = "gigachat_auth_url";
// botocore's NoCredentialsError text, which LiteLLM lets through when a Bedrock runtime call has no Bearer token
const NO_CREDENTIALS: &str = "Unable to locate credentials";
// BedrockMantleAuthMixin.sign_request wraps the same failure in this ValueError
const MANTLE_NO_CREDENTIALS: &str = "Bedrock Mantle auth failed: no Bearer token and no usable AWS \
     credentials. Set BEDROCK_MANTLE_API_KEY (or AWS_BEARER_TOKEN_BEDROCK) or pass api_key for \
     Bearer auth, or provide AWS credentials (IAM role / access key / profile / web identity) for \
     SigV4.";

pub(crate) struct AuthRequest<'call> {
    pub api_key: Option<&'call SecretString>,
    pub params: &'call Object,
    pub base: &'call str,
    pub http: &'call ProxiedClient,
}

pub(crate) struct Auth {
    pub headers: Headers,
    pub api_key: Option<SecretString>,
}

pub(crate) async fn validate(
    tag: PassthroughConfigTag,
    request: &AuthRequest<'_>,
) -> Result<Auth, ConfigError> {
    let api_key = provider_api_key(tag, request.api_key)?;
    let headers = provider_headers(tag, request, api_key.as_ref()).await?;
    Ok(Auth { headers, api_key })
}

// provider_config.get_api_key: VLLM always answers None and Bedrock hands the key back unchanged
fn provider_api_key(
    tag: PassthroughConfigTag,
    api_key: Option<&SecretString>,
) -> Result<Option<SecretString>, ConfigError> {
    match tag {
        PassthroughConfigTag::VLLM => Ok(None),
        PassthroughConfigTag::Bedrock | PassthroughConfigTag::BedrockMantle => Ok(api_key.cloned()),
        PassthroughConfigTag::Azure => {
            let config: AzurePassthroughConfig = AzurePassthroughConfig::default();
            Ok(config.get_api_key(api_key).cloned())
        }
        PassthroughConfigTag::AzureAI => Ok(filled(api_key).cloned()),
        PassthroughConfigTag::GigaChat => Ok(GigachatPassthroughConfig::default()
            .get_api_key(api_key)
            .cloned()),
        PassthroughConfigTag::Watsonx => watsonx_key(api_key).map(Some),
        PassthroughConfigTag::NvidiaNim => Ok(NvidiaNimPassthroughConfig::default()
            .get_api_key(api_key)
            .cloned()),
    }
}

// WatsonxPassthroughConfig.get_api_key reaches the credentials helper, so a missing key refuses here
fn watsonx_key(api_key: Option<&SecretString>) -> Result<SecretString, ConfigError> {
    let environment = WatsonxEnvironment::default();
    WatsonxPassthroughConfig {
        environment: &environment,
    }
    .get_api_key(api_key)
    .map_err(watsonx_error)
}

fn watsonx_error(error: WatsonxPassthroughError) -> ConfigError {
    ConfigError::from_status(error.status(), error.to_string())
}

// VLLM, Watsonx, Bedrock and Mantle add no auth header of their own: Mantle's bearer is set while signing
async fn provider_headers(
    tag: PassthroughConfigTag,
    request: &AuthRequest<'_>,
    api_key: Option<&SecretString>,
) -> Result<Headers, ConfigError> {
    match tag {
        PassthroughConfigTag::VLLM
        | PassthroughConfigTag::Bedrock
        | PassthroughConfigTag::BedrockMantle
        | PassthroughConfigTag::Watsonx => Ok(Headers::new()),
        PassthroughConfigTag::Azure => azure_headers(request, api_key),
        PassthroughConfigTag::AzureAI => azure_ai_headers(request, api_key),
        PassthroughConfigTag::GigaChat => gigachat_headers(request, api_key).await,
        PassthroughConfigTag::NvidiaNim => Ok(nvidia_nim_validate(Headers::new(), api_key)),
    }
}

// GenericLiteLLMParams.model_validate(litellm_params): the Entra fields come from the deployment params
fn deployment_settings(params: &Object) -> Result<DeploymentSettings, ConfigError> {
    Ok(DeploymentSettings {
        azure_ad_token: optional_secret(params, "azure_ad_token")?,
        tenant_id: optional_text(params, "tenant_id")?,
        client_id: optional_text(params, "client_id")?,
        client_secret: optional_secret(params, "client_secret")?,
        azure_scope: optional_text(params, "azure_scope")?.unwrap_or_default(),
        azure_username: optional_text(params, "azure_username")?,
        azure_password: optional_secret(params, "azure_password")?,
    })
}

fn optional_secret(params: &Object, name: &str) -> Result<Option<SecretString>, ConfigError> {
    let text = optional_text(params, name)?;
    Ok(text.map(SecretString::from))
}

fn azure_headers(
    request: &AuthRequest<'_>,
    api_key: Option<&SecretString>,
) -> Result<Headers, ConfigError> {
    let credentials = deployment_settings(request.params)?;
    azure_validate(AzurePassthroughEnvironment {
        headers: Headers::new(),
        api_key,
        params_key: api_key,
        configured_key: None,
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &NoSources,
    })
    .map_err(azure_error)
}

fn azure_error(error: AzurePassthroughError) -> ConfigError {
    let message = error.to_string();
    match error {
        AzurePassthroughError::Provider(provider) => ConfigError::Provider(provider),
        AzurePassthroughError::Token(token) => ConfigError::from_status(token.status(), message),
        AzurePassthroughError::MissingApiBase
        | AzurePassthroughError::Url(_)
        | AzurePassthroughError::Passthrough(_) => ConfigError::value_error(message),
    }
}

fn azure_ai_headers(
    request: &AuthRequest<'_>,
    api_key: Option<&SecretString>,
) -> Result<Headers, ConfigError> {
    let environment = BTreeMap::new();
    let params = prompts_object(request.params)?;
    azure_ai_validate(ValidateEnvironmentRequest {
        headers: Headers::new(),
        api_key,
        params: Some(&params),
        api_base: Some(request.base),
        environment: &environment,
        refresh_enabled: false,
        sources: &NoSources,
    })
    .map_err(azure_ai_error)
}

// get_azure_ai_auth_headers reads litellm_params through the prompts JSON model, the crate that owns it
fn prompts_object(params: &Object) -> Result<PromptsObject, ConfigError> {
    let value = serde_json::to_value(params)
        .map_err(|error| ConfigError::value_error(error.to_string()))?;
    serde_json::from_value(value).map_err(|error| ConfigError::value_error(error.to_string()))
}

fn azure_ai_error(error: CredentialsError) -> ConfigError {
    let message = error.to_string();
    let status = match &error {
        CredentialsError::Token(token) => token.status(),
        CredentialsError::Source(failure) => failure.status,
        CredentialsError::MissingCredentials { .. }
        | CredentialsError::AgentTokenUnresolved { .. }
        | CredentialsError::InvalidParam { .. }
        | CredentialsError::TokenNotString { .. }
        | CredentialsError::ProviderFailed { .. }
        | CredentialsError::Secret(_) => None,
    };
    ConfigError::from_status(status, message)
}

async fn gigachat_headers(
    request: &AuthRequest<'_>,
    api_key: Option<&SecretString>,
) -> Result<Headers, ConfigError> {
    let bearer = GigachatTokenBearer {
        authenticator: GigachatAuthenticator::new(
            GigachatSettings::default(),
            request.http.client.clone(),
            SystemRuntime,
        ),
        request: gigachat_token_request(request.params)?,
    };
    gigachat_validate(Headers::new(), api_key, &bearer)
        .await
        .map_err(gigachat_error)
}

// get_access_token reads these keys off litellm_params before it looks at any credential
fn gigachat_token_request(params: &Object) -> Result<GigachatTokenRequest, ConfigError> {
    let access_token = optional_text(params, ACCESS_TOKEN_PARAM)?.map(SecretString::from);
    let scope = optional_text(params, SCOPE_PARAM)?;
    let auth_url = optional_text(params, AUTH_URL_PARAM)?;
    Ok(GigachatTokenRequest {
        access_token,
        credentials: None,
        scope,
        auth_url,
    })
}

fn gigachat_error(error: GigachatPassthroughError) -> ConfigError {
    let message = error.to_string();
    match error {
        GigachatPassthroughError::Provider(provider) => ConfigError::Provider(provider),
        GigachatPassthroughError::Token(token) => ConfigError::message(token.status_code, message),
        GigachatPassthroughError::Passthrough(source) => ConfigError::Passthrough(source),
    }
}

// The merged authenticator owns the OAuth flow; the call only supplies the parameters it reads
struct GigachatTokenBearer {
    authenticator: GigachatAuthenticator<SystemRuntime>,
    request: GigachatTokenRequest,
}

#[async_trait]
impl GigachatBearer for GigachatTokenBearer {
    async fn access_token(
        &self,
        credentials: Option<&SecretString>,
    ) -> Result<SecretString, GigaChatError> {
        let mut request = self.request.clone();
        request.credentials = credentials.cloned();
        self.authenticator
            .access_token(&request)
            .await
            .map_err(|error| GigaChatError {
                status_code: error.status_code().as_u16(),
                message: error.to_string(),
                headers: None,
                source: None,
            })
    }
}

// provider_config.sign_request: only the two Bedrock variants touch the headers or the body
pub(crate) fn sign_request(
    tag: PassthroughConfigTag,
    headers: Headers,
    context: &PassthroughSigning<'_>,
) -> Result<(Headers, Option<Vec<u8>>), ConfigError> {
    match tag {
        PassthroughConfigTag::Bedrock => Err(ConfigError::value_error(NO_CREDENTIALS.to_owned())),
        PassthroughConfigTag::BedrockMantle => mantle_signed(headers, context),
        PassthroughConfigTag::VLLM
        | PassthroughConfigTag::Azure
        | PassthroughConfigTag::AzureAI
        | PassthroughConfigTag::GigaChat
        | PassthroughConfigTag::Watsonx
        | PassthroughConfigTag::NvidiaNim => Ok((headers, None)),
    }
}

// A Bearer token ends BedrockBase._sign_request before SigV4, so the signed body is the call's only body
fn mantle_signed(
    headers: Headers,
    context: &PassthroughSigning<'_>,
) -> Result<(Headers, Option<Vec<u8>>), ConfigError> {
    let Some(token) = mantle_bearer(context.litellm_params) else {
        return Err(ConfigError::value_error(MANTLE_NO_CREDENTIALS.to_owned()));
    };
    let body = context.request_data.map_or_else(empty_body, signed_body);
    let mut signed = headers;
    signed.insert(CONTENT_TYPE.to_owned(), JSON_CONTENT.to_owned());
    signed.insert(
        AUTHORIZATION.to_owned(),
        format!("Bearer {}", token.expose_secret()),
    );
    Ok((signed, Some(body)))
}

// resolve_mantle_bearer_token reads the deployment key first and falls back to the process environment
fn mantle_bearer(params: &Object) -> Option<SecretString> {
    params
        .get("api_key")
        .and_then(JsonValue::as_str)
        .map(SecretString::from)
}

// json.dumps(request_data or {}), in Python's default separators
fn signed_body(data: &Object) -> Vec<u8> {
    dumps_object(&json_object(data)).into_bytes()
}

fn empty_body() -> Vec<u8> {
    signed_body(&Object::new())
}

fn filled(api_key: Option<&SecretString>) -> Option<&SecretString> {
    api_key.filter(|key| !key.expose_secret().is_empty())
}

fn optional_text(params: &Object, name: &str) -> Result<Option<String>, ConfigError> {
    match params.get(name) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => Ok(Some(text.clone())),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(ConfigError::value_error(format!(
            "litellm_params.{name} must be a string"
        ))),
    }
}
