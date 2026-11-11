use std::time::{Duration, TryFromFloatSecsError};

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::controls::call_params::{DropParams, normalize_drop_params};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::headers::affinity::{AffinityError, validate_affinity_header};
use betterlite_openai::chat::deployment::{ChatDeployment, Provider as DeploymentProvider};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use url::Url;

use crate::operation::OperationError;
use crate::provider::Provider;

// LiteLLM's DEFAULT_REQUEST_TIMEOUT_SECONDS
const DEFAULT_TIMEOUT_SECONDS: f64 = 6000.0;
const DEFAULT_OPENAI_BASE: &str = "https://api.openai.com/v1";
const RESERVED_PARAMS: [&str; 3] = ["self", "params", "__class__"];

// The trusted values a deployment hands to its provider; none of them come from the request body.
#[derive(Debug, Clone)]
pub struct Deployment {
    pub model: String,
    pub api_base: String,
    pub endpoint: Url,
    pub api_key: Option<SecretString>,
    pub authorization: Option<SecretString>,
    pub organization: Option<String>,
    pub timeout: Duration,
    pub max_retries: Option<u32>,
    pub drop_params: Option<DropParams>,
    pub provider_affinity_header: Option<String>,
}

impl Deployment {
    #[must_use]
    pub fn from_chat(deployment: &ChatDeployment) -> Self {
        let model = match &deployment.provider {
            DeploymentProvider::OpenAi { model } => model.clone(),
            DeploymentProvider::Unsupported { model } => stripped_model(model),
        };
        Self {
            model,
            api_base: deployment.api_base.clone(),
            endpoint: deployment.endpoint.clone(),
            api_key: deployment.api_key.clone(),
            authorization: deployment.authorization.clone(),
            organization: deployment.organization.clone(),
            timeout: deployment.timeout,
            max_retries: None,
            drop_params: None,
            provider_affinity_header: None,
        }
    }

    pub fn from_params(mut params: IndexMap<String, JsonValue>) -> Result<Self, DeploymentError> {
        for reserved in RESERVED_PARAMS {
            params.shift_remove(reserved);
        }
        let model =
            text(params.shift_remove("model"), "model")?.ok_or(DeploymentError::MissingModel)?;
        let api_base = text(params.shift_remove("api_base"), "api_base")?
            .unwrap_or_else(|| String::from(DEFAULT_OPENAI_BASE));
        let url = Url::parse(&api_base).map_err(|source| DeploymentError::ApiBase {
            value: api_base.clone(),
            source,
        })?;
        let endpoint = chat_endpoint(url);
        let api_key = text(params.shift_remove("api_key"), "api_key")?.map(SecretString::from);
        let organization = text(params.shift_remove("organization"), "organization")?;
        let timeout = seconds(params.shift_remove("timeout"))?;
        let max_retries = retries(params.shift_remove("max_retries"))?;
        let drop_params = drop_params(params.shift_remove("drop_params"));
        let provider_affinity_header =
            affinity_header(params.shift_remove("provider_affinity_header"))?;
        Ok(Self {
            model,
            api_base,
            endpoint,
            api_key,
            authorization: None,
            organization,
            timeout,
            max_retries,
            drop_params,
            provider_affinity_header,
        })
    }

    // url userinfo becomes a Basic header that replaces the bearer, exactly like HTTPX's auth flow
    #[must_use]
    pub fn credential(&self) -> Option<SecretString> {
        self.authorization.clone().or_else(|| {
            self.api_key
                .as_ref()
                .map(|key| SecretString::from(format!("Bearer {}", key.expose_secret())))
        })
    }

    #[must_use]
    pub fn endpoint_for(&self, resource: &str) -> Url {
        let mut url = self.endpoint.clone();
        let path = format!(
            "{}/{}",
            url.path().trim_end_matches("/chat/completions"),
            resource
        );
        url.set_path(&path);
        url
    }
}

// only a prefix that names a provider comes off; an org/model name keeps its slash
fn stripped_model(litellm_model: &str) -> String {
    litellm_model
        .split_once('/')
        .filter(|(slug, _model)| Provider::from_slug(slug).is_ok())
        .map_or_else(|| litellm_model.to_owned(), |(_, model)| model.to_owned())
}

fn chat_endpoint(mut url: Url) -> Url {
    let path = format!("{}/chat/completions", url.path().trim_end_matches('/'));
    url.set_path(&path);
    url
}

fn text(value: Option<JsonValue>, field: &'static str) -> Result<Option<String>, DeploymentError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Text(text)) => Ok(Some(text)),
        Some(_other) => Err(DeploymentError::FieldType {
            field,
            expected: "a string",
        }),
    }
}

fn seconds(value: Option<JsonValue>) -> Result<Duration, DeploymentError> {
    let given = match value {
        None | Some(JsonValue::Null) => {
            return Duration::try_from_secs_f64(DEFAULT_TIMEOUT_SECONDS).map_err(|source| {
                DeploymentError::Timeout {
                    value: DEFAULT_TIMEOUT_SECONDS,
                    source,
                }
            });
        }
        Some(given) => given,
    };
    let JsonValue::Number(number) = &given else {
        return Err(DeploymentError::FieldType {
            field: "timeout",
            expected: "a number of seconds",
        });
    };
    let Some(seconds) = number.as_f64() else {
        return Err(DeploymentError::FieldType {
            field: "timeout",
            expected: "a number of seconds",
        });
    };
    Duration::try_from_secs_f64(seconds).map_err(|source| DeploymentError::Timeout {
        value: seconds,
        source,
    })
}

fn retries(value: Option<JsonValue>) -> Result<Option<u32>, DeploymentError> {
    let Some(given) = value else {
        return Ok(None);
    };
    match given {
        JsonValue::Null => Ok(None),
        JsonValue::Number(number) => retry_count(&number),
        JsonValue::Text(text) => retry_text(text),
        JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(DeploymentError::FieldType {
                field: "max_retries",
                expected: "an integer",
            })
        }
    }
}

fn retry_count(number: &serde_json::Number) -> Result<Option<u32>, DeploymentError> {
    number
        .as_u64()
        .and_then(|count| u32::try_from(count).ok())
        .map(Some)
        .ok_or_else(|| DeploymentError::MaxRetries {
            value: number.to_string(),
        })
}

fn retry_text(text: String) -> Result<Option<u32>, DeploymentError> {
    text.parse::<u32>()
        .map(Some)
        .map_err(|_invalid| DeploymentError::MaxRetries { value: text })
}

fn drop_params(value: Option<JsonValue>) -> Option<DropParams> {
    let given = match value {
        None | Some(JsonValue::Null) => return None,
        Some(JsonValue::Bool(flag)) => DropParams::Flag(flag),
        Some(JsonValue::Text(text)) => DropParams::Text(text),
        Some(other) => {
            tracing::warn!(
                value = ?other,
                "drop_params is not a flag value, treating it as unset"
            );
            return None;
        }
    };
    // a flag value wins, and any other string stays as written for the later policy step
    Some(normalize_drop_params(Some(&given)).map_or(given, DropParams::Flag))
}

fn affinity_header(value: Option<JsonValue>) -> Result<Option<String>, DeploymentError> {
    let header = text(value, "provider_affinity_header")?;
    if let Some(name) = &header {
        validate_affinity_header(name).map_err(|source| {
            DeploymentError::ProviderAffinityHeader {
                value: name.clone(),
                source,
            }
        })?;
    }
    Ok(header)
}

#[derive(Debug, Error)]
pub enum DeploymentError {
    #[error("the deployment params name no model")]
    MissingModel,
    #[error("deployment field {field} must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("deployment max_retries {value:?} is not an integer")]
    MaxRetries { value: String },
    #[error("deployment api_base {value:?} is not a URL: {source}")]
    ApiBase {
        value: String,
        #[source]
        source: url::ParseError,
    },
    #[error("deployment timeout {value} seconds cannot be represented")]
    Timeout {
        value: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("deployment provider_affinity_header {value:?} cannot be used: {source}")]
    ProviderAffinityHeader {
        value: String,
        #[source]
        source: AffinityError,
    },
}

impl From<TransportError> for OperationError {
    fn from(source: TransportError) -> Self {
        match source {
            TransportError::ConnectTimeout(_) | TransportError::Deadline(_) => Self::Timeout,
            TransportError::BodyTooLarge { limit } | TransportError::HeadersTooLarge { limit } => {
                Self::TooLarge { limit }
            }
            TransportError::Rejected(_)
            | TransportError::Build(_)
            | TransportError::Connect(_)
            | TransportError::Request(_)
            | TransportError::Body(_)
            | TransportError::InvalidHeader { .. }
            | TransportError::MultipartContentType { .. }
            | TransportError::MultipartBoundaryHeader { .. }
            | TransportError::MultipartBoundary { .. }
            | TransportError::Mock(_)
            | TransportError::Closed
            | TransportError::DrainTimeout { .. }
            | TransportError::TooManyRedirects { .. }
            | TransportError::RedirectTarget { .. }
            | TransportError::RedirectUrl { .. }
            | TransportError::RedirectUserinfo
            | TransportError::RedirectScheme { .. }
            | TransportError::InsecureRedirect
            | TransportError::RedirectBody => Self::Http { source },
        }
    }
}
