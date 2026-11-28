use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::gigachat_auth::Raw as GigachatRaw;
use betterlite_config::sections::settings::JsonValue;
use betterlite_context::resource_config_selection::passthrough::PassthroughConfigTag;
use betterlite_http::gigachat_tokens::authenticator::GigachatAuthenticator;
use betterlite_http::gigachat_tokens::request::GigachatTokenRequest;
use betterlite_http::gigachat_tokens::runtime::SystemRuntime;
use betterlite_http::gigachat_tokens::tokens::GigachatTokens as _;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::python_json::dumps_object;
use betterlite_openai::gigachat_chat::config::GigachatBearer;
use betterlite_openai::gigachat_chat::error::GigaChatError;
use betterlite_openai::gigachat_passthrough::{
    GigachatPassthroughConfig, validate_environment as gigachat_validate,
};
use betterlite_openai::nvidia_nim_passthrough::{
    NvidiaNimPassthroughConfig, validate_environment as nvidia_nim_validate,
};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::types::PassthroughSigning;
use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::error::WatsonxError;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use betterlite_openai::watsonx_passthrough::{WatsonxPassthroughConfig, WatsonxPassthroughError};
use http::header::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};

use self::providers::{AzureKind, azure_headers, gigachat_error, status_error};
use super::config::{ConfigError, PassthroughCallConfig};
use super::logging::json_object;
use super::wire::header_text;

mod providers;

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

// step 8: provider_api_key = config.get_api_key(api_key), then validate_environment's headers
pub(crate) async fn validate(
    config: &PassthroughCallConfig,
    api_key: Option<&SecretString>,
    params: &Object,
    base: &str,
    http: &ProxiedClient,
) -> Result<Headers, ConfigError> {
    let environment = config.environment.as_ref();
    match config.tag {
        PassthroughConfigTag::VLLM
        | PassthroughConfigTag::Bedrock
        | PassthroughConfigTag::BedrockMantle => Ok(Headers::new()),
        PassthroughConfigTag::Watsonx => {
            watsonx_headers(WatsonxEnvironment::from_lookup(environment), api_key, http).await
        }
        PassthroughConfigTag::Azure => {
            azure_headers(AzureKind::Key, api_key, params, base, environment)
        }
        PassthroughConfigTag::AzureAI => {
            azure_headers(AzureKind::Entra, api_key, params, base, environment)
        }
        PassthroughConfigTag::GigaChat => {
            gigachat_headers(api_key, params, environment, http).await
        }
        PassthroughConfigTag::NvidiaNim => {
            // get_llm_provider already applied NVIDIA_NIM_API_BASE and NVIDIA_NIM_API_KEY, so the config's env fields stay empty
            let nim: NvidiaNimPassthroughConfig = NvidiaNimPassthroughConfig::default();
            Ok(nvidia_nim_validate(
                Headers::new(),
                nim.get_api_key(api_key),
            ))
        }
    }
}

// WatsonxPassthroughConfig's validate_environment: content type and accept, then the Bearer or ZenApiKey header
pub(super) async fn watsonx_headers(
    environment: WatsonxEnvironment,
    api_key: Option<&SecretString>,
    http: &ProxiedClient,
) -> Result<Headers, ConfigError> {
    let config = WatsonxPassthroughConfig {
        environment: &environment,
    };
    let provider_key = config.get_api_key(api_key)?;
    let auth = WatsonxAuth::new(environment, http.clone(), Arc::new(epoch_seconds))?;
    let mut params = WatsonxParams::new();
    let headers = auth
        .validate_environment(HeaderMap::new(), &mut params, Some(&provider_key))
        .await?;
    Ok(headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect())
}

// the merged config's status(): only its provider API failure carries an upstream status
impl From<WatsonxPassthroughError> for ConfigError {
    fn from(error: WatsonxPassthroughError) -> Self {
        status_error(error.status(), error.to_string())
    }
}

// LiteLLM re-raises the IAM httpx.HTTPStatusError unchanged, so its 401 reaches _handle_error as the status
impl From<WatsonxError> for ConfigError {
    fn from(error: WatsonxError) -> Self {
        let message = error.to_string();
        match error {
            WatsonxError::IamStatus { status, .. } => Self::Message { status, message },
            other @ (WatsonxError::Api { .. }
            | WatsonxError::IamApiKeyRequired
            | WatsonxError::InvalidField { .. }
            | WatsonxError::InvalidHeader { .. }
            | WatsonxError::IamUrl(_)
            | WatsonxError::IamTransport(_)
            | WatsonxError::IamReply(_)
            | WatsonxError::IamReplyField { .. }
            | WatsonxError::Cache(_)) => Self::from(WatsonxPassthroughError::from(other)),
        }
    }
}

// the wall clock only seeds the IAM token cache, whose TTL no test in this module can read
#[cfg_attr(test, mutants::skip)]
fn epoch_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

// GigaChat's validate_environment reads its OAuth parameters from litellm_params, then mints a token
async fn gigachat_headers(
    api_key: Option<&SecretString>,
    params: &Object,
    environment: &BTreeMap<String, String>,
    http: &ProxiedClient,
) -> Result<Headers, ConfigError> {
    let settings = GigachatRaw::default()
        .convert(environment)
        .map_err(|error| ConfigError::ValueError {
            message: error.to_string(),
        })?;
    // get_llm_provider already applied GIGACHAT_API_BASE and GIGACHAT_API_KEY, so the config's env fields stay empty
    let config: GigachatPassthroughConfig = GigachatPassthroughConfig::default();
    let bearer = GigachatTokenBearer {
        authenticator: GigachatAuthenticator::new(settings, http.client.clone(), SystemRuntime),
        request: GigachatTokenRequest {
            access_token: optional_text(params, ACCESS_TOKEN_PARAM)?.map(SecretString::from),
            credentials: None,
            scope: optional_text(params, SCOPE_PARAM)?,
            auth_url: optional_text(params, AUTH_URL_PARAM)?,
        },
    };
    gigachat_validate(Headers::new(), config.get_api_key(api_key), &bearer)
        .await
        .map_err(gigachat_error)
}

// provider_config.sign_request: only the two Bedrock variants touch the headers or the body
pub(crate) fn sign_request(
    tag: PassthroughConfigTag,
    headers: Headers,
    context: &PassthroughSigning<'_>,
) -> Result<(Headers, Option<Vec<u8>>), ConfigError> {
    match tag {
        PassthroughConfigTag::Bedrock => Err(ConfigError::ValueError {
            message: NO_CREDENTIALS.to_owned(),
        }),
        PassthroughConfigTag::BedrockMantle => {
            let token = context
                .litellm_params
                .get("api_key")
                .and_then(JsonValue::as_str)
                .map(SecretString::from);
            let Some(secret) = token else {
                return Err(ConfigError::ValueError {
                    message: MANTLE_NO_CREDENTIALS.to_owned(),
                });
            };
            let body = context.request_data.map_or_else(
                || dumps_object(&json_object(&Object::new())).into_bytes(),
                |data| dumps_object(&json_object(data)).into_bytes(),
            );
            let mut signed = headers;
            signed.insert(CONTENT_TYPE.to_owned(), JSON_CONTENT.to_owned());
            signed.insert(
                AUTHORIZATION.to_owned(),
                format!("Bearer {}", secret.expose_secret()),
            );
            Ok((signed, Some(body)))
        }
        PassthroughConfigTag::VLLM
        | PassthroughConfigTag::Azure
        | PassthroughConfigTag::AzureAI
        | PassthroughConfigTag::GigaChat
        | PassthroughConfigTag::Watsonx
        | PassthroughConfigTag::NvidiaNim => Ok((headers, None)),
    }
}

// The merged authenticator owns the OAuth flow; the call only supplies the parameters it reads
struct GigachatTokenBearer {
    authenticator: GigachatAuthenticator<SystemRuntime>,
    request: GigachatTokenRequest,
}

#[async_trait]
impl GigachatBearer for GigachatTokenBearer {
    // the OAuth round trip needs a live GigaChat token endpoint; betterlite-http tests it
    #[cfg_attr(test, mutants::skip)]
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

pub(super) fn optional_text(params: &Object, name: &str) -> Result<Option<String>, ConfigError> {
    match params.get(name) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => Ok(Some(text.clone())),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(ConfigError::ValueError {
            message: format!("litellm_params.{name} must be a string"),
        }),
    }
}
