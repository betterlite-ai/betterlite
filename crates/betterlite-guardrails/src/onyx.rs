use crate::generic_guardrail_contracts::GuardrailInputType;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_env::EnvError;
use hyper::Error as HyperError;
use reqwest::StatusCode;
use reqwest::header::CONTENT_TYPE;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error as _;
use std::io::Error as IoError;
use std::io::ErrorKind::{InvalidData, InvalidInput, UnexpectedEof};
use std::num::TryFromIntError;
use std::time::{Duration, TryFromFloatSecsError};
use thiserror::Error;
use uuid::Uuid;
pub const SUPPORTED_EVENT_HOOKS: [&str; 3] = ["pre_call", "during_call", "post_call"];
#[derive(Debug, Error)]
pub enum OnyxGuardrailError {
    #[error("ONYX_API_KEY environment variable is not set")]
    MissingApiKey,
    #[error("ONYX_TIMEOUT must be an integer number of seconds, got {1:?}")]
    TimeoutNotInteger(#[source] EnvError, String),
    #[error("ONYX_TIMEOUT must not be negative, got {1:?}")]
    TimeoutNegative(#[source] TryFromIntError, String),
    #[error("the Onyx Guard timeout is not a representable duration")]
    UnrepresentableTimeout { source: TryFromFloatSecsError },
    #[error("the Onyx Guard server could not be reached")]
    Transport { source: reqwest::Error },
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
    allowed: Option<bool>,
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
            .or_else(|| lookup.lookup("ONYX_API_KEY").map(SecretString::from))
            .filter(|key| !key.expose_secret().is_empty())
            .ok_or(OnyxGuardrailError::MissingApiKey)?;
        let api_base = configured_base
            .or_else(|| {
                lookup
                    .lookup("ONYX_API_BASE")
                    .filter(|base| !base.is_empty())
            })
            .unwrap_or_else(|| "https://ai-guard.onyx.security".to_owned());
        let timeout = match configured_timeout {
            Some(seconds) => Duration::try_from_secs_f64(seconds)
                .map_err(|source| OnyxGuardrailError::UnrepresentableTimeout { source })?,
            None => environment_timeout(lookup)?,
        };
        let deadline = timeout
            .saturating_mul(3)
            .saturating_add(Duration::from_secs(5));
        let client = client_builder
            .connect_timeout(Duration::from_secs(5))
            .read_timeout(timeout)
            .timeout(deadline)
            .build()
            .map_err(|source| OnyxGuardrailError::Transport { source })?;
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
        let payload = payload_for(request_data, input_type, &conversation_id);
        match self
            .validate_with_guard_server(&payload, input_type, &conversation_id)
            .await
        {
            Ok(()) => Ok(inputs),
            Err(error @ OnyxGuardrailError::Blocked { .. }) => Err(error),
            Err(error) => {
                tracing::error!(error = %error, "Error in apply_guardrail guard");
                Ok(inputs)
            }
        }
    }
    async fn attempt(
        &self,
        endpoint: &str,
        request: &OnyxRequest<'_>,
    ) -> Result<OnyxVerdict, OnyxGuardrailError> {
        let mut response = self
            .client
            .post(endpoint)
            .header(CONTENT_TYPE, "application/json")
            .json(request)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            body.extend_from_slice(&chunk);
            if body.len() > 1024 * 1024 {
                return Err(OnyxGuardrailError::ResponseTooLarge { limit: 1024 * 1024 });
            }
        }
        if !status.is_success() {
            return Err(OnyxGuardrailError::HttpStatus { status });
        }
        serde_json::from_slice(&body).map_err(|source| OnyxGuardrailError::ResponseJson { source })
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
        let verdict = match self.attempt(&endpoint, &request).await {
            Ok(verdict) => verdict,
            Err(OnyxGuardrailError::Transport { source }) if retryable(&source) => {
                self.attempt(&endpoint, &request).await?
            }
            Err(error) => return Err(error),
        };
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
// LiteLLM retries a ConnectError or RemoteProtocolError once; a timeout or a reset is not retried.
fn retryable(error: &reqwest::Error) -> bool {
    if error.is_timeout() {
        return false;
    }
    if error.is_connect() {
        return true;
    }
    let mut source = error.source();
    let mut under_hyper = false;
    while let Some(cause) = source {
        if let Some(hyper) = cause.downcast_ref::<HyperError>() {
            if hyper.is_incomplete_message() || hyper.is_parse() {
                return true;
            }
            under_hyper = true;
        }
        if under_hyper
            && cause
                .downcast_ref::<IoError>()
                .is_some_and(|io| matches!(io.kind(), UnexpectedEof | InvalidInput | InvalidData))
        {
            return true;
        }
        source = cause.source();
    }
    false
}
fn environment_timeout<L>(lookup: &L) -> Result<Duration, OnyxGuardrailError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup("ONYX_TIMEOUT") else {
        return Ok(Duration::from_secs(10));
    };
    let parsed = betterlite_env::number::<i64>("ONYX_TIMEOUT", &text)
        .map_err(|source| OnyxGuardrailError::TimeoutNotInteger(source, text.clone()))?;
    let seconds = parsed
        .map(u64::try_from)
        .transpose()
        .map_err(|source| OnyxGuardrailError::TimeoutNegative(source, text))?;
    Ok(seconds.map_or(Duration::from_secs(10), Duration::from_secs))
}
fn payload_field(request_data: &BTreeMap<String, GuardrailValue>, name: &str) -> GuardrailValue {
    request_data
        .get(name)
        .filter(|value| !matches!(value, GuardrailValue::Null))
        .cloned()
        .unwrap_or_else(|| GuardrailValue::Object(BTreeMap::new()))
}
fn payload_for(
    request_data: &BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    conversation_id: &str,
) -> GuardrailValue {
    match input_type {
        GuardrailInputType::Request => payload_field(request_data, "proxy_server_request"),
        GuardrailInputType::Response if !model_response_accepts(request_data) => {
            tracing::error!(
                conversation_id,
                input_type = ?input_type,
                "Error in converting request_data to ModelResponse"
            );
            GuardrailValue::Object(request_data.clone())
        }
        GuardrailInputType::Response => payload_field(request_data, "response"),
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
    OnyxGuardrailError::Transport {
        source: source.without_url(),
    }
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct OnyxGuardrailConfigModel {
    pub optional_params: Option<BTreeMap<String, GuardrailValue>>,
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub timeout: Option<f64>,
}
impl GuardrailConfigModelIdentity for OnyxGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Onyx Guardrail"
    }
}
