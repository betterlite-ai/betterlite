use std::collections::BTreeMap;
use std::time::{Duration, TryFromFloatSecsError};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use reqwest::header::CONTENT_TYPE;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::generic_guardrail_contracts::GuardrailInputType;

pub const DEFAULT_API_BASE: &str = "https://ai-guard.onyx.security";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const ONYX_API_KEY_VARIABLE: &str = "ONYX_API_KEY";
pub const ONYX_API_BASE_VARIABLE: &str = "ONYX_API_BASE";
pub const ONYX_TIMEOUT_VARIABLE: &str = "ONYX_TIMEOUT";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const EMPTY_OBJECT: GuardrailValue = GuardrailValue::Object(BTreeMap::new());
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventHook {
    PreCall,
    DuringCall,
    PostCall,
}
pub const SUPPORTED_EVENT_HOOKS: [EventHook; 3] = [
    EventHook::PreCall,
    EventHook::DuringCall,
    EventHook::PostCall,
];

#[derive(Debug, Error)]
pub enum OnyxGuardrailError {
    #[error("ONYX_API_KEY environment variable is not set")]
    MissingApiKey,
    #[error("ONYX_TIMEOUT must be a non-negative integer number of seconds, got {value:?}")]
    InvalidTimeout { value: String },
    #[error("the Onyx Guard timeout is not a representable duration")]
    UnrepresentableTimeout { source: TryFromFloatSecsError },
    #[error("the Onyx Guard HTTP client could not be built")]
    Client { source: reqwest::Error },
    #[error("the Onyx Guard request timed out")]
    Timeout { source: reqwest::Error },
    #[error("the Onyx Guard server could not be reached")]
    Request { source: reqwest::Error },
    #[error("the Onyx Guard server returned status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Onyx Guard response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Onyx Guard response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("Request blocked by Onyx Guard. Violations: {message}.")]
    Blocked { message: String },
}
impl OnyxGuardrailError {
    // LiteLLM answers a block with HTTPException(400); every other failure is logged and passed through.
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        if matches!(self, Self::Blocked { .. }) {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[derive(Debug, Serialize)]
struct OnyxRequest<'input> {
    payload: &'input GuardrailValue,
    input_type: GuardrailInputType,
    conversation_id: &'input str,
}
#[derive(Debug, Deserialize)]
struct OnyxVerdict {
    #[serde(default)]
    allowed: Option<bool>,
    #[serde(default)]
    violated_rules: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct OnyxGuardrail {
    pub api_base: String,
    pub api_key: SecretString,
    pub timeout: Duration,
    pub client: reqwest::Client,
    pub new_id: fn() -> Uuid,
}
impl OnyxGuardrail {
    pub fn new<L>(
        configured_key: Option<SecretString>,
        configured_base: Option<String>,
        configured_timeout: Option<f64>,
        lookup: &L,
        client_builder: reqwest::ClientBuilder,
    ) -> Result<Self, OnyxGuardrailError>
    where
        L: EnvironmentLookup,
    {
        let api_key = configured_key
            .or_else(|| lookup.lookup(ONYX_API_KEY_VARIABLE).map(SecretString::from))
            .filter(|key| !key.expose_secret().is_empty())
            .ok_or(OnyxGuardrailError::MissingApiKey)?;
        let api_base = configured_base
            .or_else(|| {
                lookup
                    .lookup(ONYX_API_BASE_VARIABLE)
                    .filter(|base| !base.is_empty())
            })
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        let timeout = match configured_timeout {
            Some(seconds) => Duration::try_from_secs_f64(seconds)
                .map_err(|source| OnyxGuardrailError::UnrepresentableTimeout { source })?,
            None => environment_timeout(lookup)?,
        };
        let client = client_builder
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|source| OnyxGuardrailError::Client { source })?;
        tracing::info!(api_base = %api_base, "OnyxGuard initialized with server");
        Ok(Self {
            api_base,
            api_key,
            timeout,
            client,
            new_id: Uuid::new_v4,
        })
    }

    pub async fn apply_guardrail(
        &self,
        inputs: BTreeMap<String, GuardrailValue>,
        request_data: &BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        litellm_call_id: Option<&str>,
    ) -> Result<BTreeMap<String, GuardrailValue>, OnyxGuardrailError> {
        let conversation_id =
            litellm_call_id.map_or_else(|| (self.new_id)().to_string(), ToOwned::to_owned);
        let payload = payload_for(request_data, input_type);
        match self
            .validate_with_guard_server(&payload, input_type, &conversation_id)
            .await
        {
            Ok(()) => Ok(inputs),
            Err(error @ OnyxGuardrailError::Blocked { .. }) => Err(error),
            Err(error) => {
                tracing::error!(
                    error = %error,
                    conversation_id = %conversation_id,
                    input_type = ?input_type,
                    "Error in apply_guardrail guard"
                );
                Ok(inputs)
            }
        }
    }

    async fn validate_with_guard_server(
        &self,
        payload: &GuardrailValue,
        input_type: GuardrailInputType,
        conversation_id: &str,
    ) -> Result<(), OnyxGuardrailError> {
        let endpoint = format!(
            "{}/guard/evaluate/v1/{}/litellm",
            self.api_base,
            self.api_key.expose_secret()
        );
        let request = OnyxRequest {
            payload,
            input_type,
            conversation_id,
        };
        let response = self
            .client
            .post(endpoint)
            .header(CONTENT_TYPE, "application/json")
            .json(&request)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(OnyxGuardrailError::HttpStatus { status });
        }
        let body = read_capped_body(response).await?;
        let verdict: OnyxVerdict = serde_json::from_slice(&body)
            .map_err(|source| OnyxGuardrailError::ResponseJson { source })?;
        if verdict.allowed.unwrap_or(true) {
            return Ok(());
        }
        let message = verdict.violated_rules.as_deref().map_or_else(
            || String::from("Unknown violation"),
            |rules| rules.join(", "),
        );
        tracing::warn!(violations = %message, "Request blocked by Onyx Guard");
        Err(OnyxGuardrailError::Blocked { message })
    }
}
fn environment_timeout<L>(lookup: &L) -> Result<Duration, OnyxGuardrailError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(ONYX_TIMEOUT_VARIABLE) else {
        return Ok(DEFAULT_TIMEOUT);
    };
    let invalid = |value: &str| OnyxGuardrailError::InvalidTimeout {
        value: value.to_owned(),
    };
    let Some(parsed) = betterlite_env::number::<i64>(ONYX_TIMEOUT_VARIABLE, &text)
        .map_err(|_source| invalid(&text))?
    else {
        return Ok(DEFAULT_TIMEOUT);
    };
    let seconds = u64::try_from(parsed).map_err(|_source| invalid(&text))?;
    Ok(Duration::from_secs(seconds))
}

fn payload_for(
    request_data: &BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
) -> GuardrailValue {
    match input_type {
        GuardrailInputType::Request => request_data
            .get("proxy_server_request")
            .filter(|value| !matches!(value, GuardrailValue::Null))
            .cloned()
            .unwrap_or(EMPTY_OBJECT),
        GuardrailInputType::Response if !model_response_accepts(request_data) => {
            GuardrailValue::Object(request_data.clone())
        }
        GuardrailInputType::Response => request_data
            .get("response")
            .filter(|value| !matches!(value, GuardrailValue::Null))
            .cloned()
            .unwrap_or(EMPTY_OBJECT),
    }
}

// ModelResponse(**request_data).json() raises on these top-level fields, and the hook then sends request_data itself.
fn model_response_accepts(request_data: &BTreeMap<String, GuardrailValue>) -> bool {
    let text = |name: &str| {
        request_data
            .get(name)
            .is_none_or(|value| matches!(value, GuardrailValue::Null | GuardrailValue::Text(_)))
    };
    text("id")
        && text("model")
        && text("system_fingerprint")
        && request_data.get("created").is_none_or(|value| {
            matches!(
                value,
                GuardrailValue::Null
                    | GuardrailValue::SignedInteger(_)
                    | GuardrailValue::UnsignedInteger(_)
            )
        })
}
fn request_error(source: reqwest::Error) -> OnyxGuardrailError {
    // The endpoint path carries the API key, so the URL never reaches a log line through the error.
    let stripped = source.without_url();
    if stripped.is_timeout() {
        OnyxGuardrailError::Timeout { source: stripped }
    } else {
        OnyxGuardrailError::Request { source: stripped }
    }
}
async fn read_capped_body(mut response: reqwest::Response) -> Result<Vec<u8>, OnyxGuardrailError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(OnyxGuardrailError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
    }
    Ok(body)
}

// No Debug derive: LiteLLM's config model carries api_key as a plain string, and Debug would print it.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct OnyxGuardrailConfigModel {
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub timeout: Option<f64>,
}

impl GuardrailConfigModelIdentity for OnyxGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Onyx Guardrail"
    }
}
