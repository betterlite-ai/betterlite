use async_trait::async_trait;
use betterlite_config::sections::gigachat_auth::Settings as GigachatSettings;
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
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_passthrough::WatsonxPassthroughConfig;
use secrecy::{ExposeSecret as _, SecretString};

use self::providers::{AzureKind, azure_headers, gigachat_error, status_error};
use super::config::ConfigError;
use super::logging::json_object;

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
    tag: PassthroughConfigTag,
    api_key: Option<&SecretString>,
    params: &Object,
    base: &str,
    http: &ProxiedClient,
) -> Result<Headers, ConfigError> {
    match tag {
        PassthroughConfigTag::VLLM
        | PassthroughConfigTag::Bedrock
        | PassthroughConfigTag::BedrockMantle => Ok(Headers::new()),
        PassthroughConfigTag::Watsonx => {
            let environment = WatsonxEnvironment::default();
            WatsonxPassthroughConfig {
                environment: &environment,
            }
            .get_api_key(api_key)
            .map_err(|error| status_error(error.status(), error.to_string()))?;
            Ok(Headers::new())
        }
        PassthroughConfigTag::Azure => azure_headers(AzureKind::Key, api_key, params, base),
        PassthroughConfigTag::AzureAI => azure_headers(AzureKind::Entra, api_key, params, base),
        PassthroughConfigTag::GigaChat => gigachat_headers(api_key, params, http).await,
        PassthroughConfigTag::NvidiaNim => {
            let config: NvidiaNimPassthroughConfig = NvidiaNimPassthroughConfig::default();
            Ok(nvidia_nim_validate(
                Headers::new(),
                config.get_api_key(api_key),
            ))
        }
    }
}

// GigaChat's validate_environment reads its OAuth parameters from litellm_params, then mints a token
async fn gigachat_headers(
    api_key: Option<&SecretString>,
    params: &Object,
    http: &ProxiedClient,
) -> Result<Headers, ConfigError> {
    let config: GigachatPassthroughConfig = GigachatPassthroughConfig::default();
    let bearer = GigachatTokenBearer {
        authenticator: GigachatAuthenticator::new(
            GigachatSettings::default(),
            http.client.clone(),
            SystemRuntime,
        ),
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
