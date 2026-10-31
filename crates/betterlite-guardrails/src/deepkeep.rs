use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::deepkeep_firewall::{Configured, UnreachableFallback};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::header::{
    CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue,
};
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod transport;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inputs {
    #[serde(default)]
    pub texts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<GuardrailValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<GuardrailValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<GuardrailValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_messages: Option<Vec<GuardrailValue>>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RequestContext {
    pub request_data: BTreeMap<String, GuardrailValue>,
    pub input_type: InputType,
    pub litellm_call_id: Option<String>,
    pub litellm_trace_id: Option<String>,
    pub litellm_version: Option<String>,
    pub dynamic_params: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Request {
    pub litellm_call_id: Option<String>,
    pub litellm_trace_id: Option<String>,
    pub texts: Vec<String>,
    pub images: Option<Vec<GuardrailValue>>,
    pub tools: Option<Vec<GuardrailValue>>,
    pub tool_calls: Option<Vec<GuardrailValue>>,
    pub structured_messages: Option<Vec<GuardrailValue>>,
    pub request_data: BTreeMap<String, GuardrailValue>,
    pub litellm_version: Option<String>,
    #[serde(rename = "additional_provider_specific_params")]
    pub provider_params: BTreeMap<String, GuardrailValue>,
    pub input_type: InputType,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputType {
    #[default]
    Request,
    Response,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Response {
    #[serde(default)]
    pub action: Option<String>,
    pub texts: Option<Vec<String>>,
    pub images: Option<Vec<GuardrailValue>>,
    pub tools: Option<Vec<GuardrailValue>>,
    pub tool_calls: Option<Vec<GuardrailValue>>,
    pub structured_messages: Option<Vec<GuardrailValue>>,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Error)]
pub enum DeepKeepError {
    #[error("DeepKeep blocked the request: {reason}")]
    Blocked { reason: String },
    #[error("DeepKeep API returned HTTP {status}: {body}")]
    Api {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("DeepKeep response was invalid: {source}")]
    ResponseJson { source: serde_json::Error },
    #[error("DeepKeep request failed: {source}")]
    Request { source: reqwest::Error },
    #[error("DeepKeep request timed out: {source}")]
    Timeout { source: reqwest::Error },
    #[error("DeepKeep request could not be built: {source}")]
    RequestBuild { source: reqwest::Error },
    #[error("DeepKeep header {name} is invalid: {source}")]
    InvalidHeader {
        name: &'static str,
        source: InvalidHeaderValue,
    },
    #[error("DeepKeep header name {name} is invalid: {source}")]
    InvalidHeaderName {
        name: String,
        source: InvalidHeaderName,
    },
}

#[derive(Debug, Clone)]
pub struct DeepKeepGuardrail {
    pub settings: Configured,
    pub client: reqwest::Client,
    pub timeout: Duration,
}

impl DeepKeepGuardrail {
    pub fn request_headers(&self) -> Result<HeaderMap, DeepKeepError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let mut api_key =
            HeaderValue::from_str(self.settings.api_key.expose_secret()).map_err(|source| {
                DeepKeepError::InvalidHeader {
                    name: "X-API-Key",
                    source,
                }
            })?;
        api_key.set_sensitive(true);
        headers.insert(HeaderName::from_static("x-api-key"), api_key);
        for (name, value) in &self.settings.extra_headers {
            let header_name =
                HeaderName::try_from(name).map_err(|source| DeepKeepError::InvalidHeaderName {
                    name: name.clone(),
                    source,
                })?;
            let mut header_value =
                HeaderValue::from_str(value.expose_secret()).map_err(|source| {
                    DeepKeepError::InvalidHeader {
                        name: "extra header",
                        source,
                    }
                })?;
            header_value.set_sensitive(true);
            headers.insert(header_name, header_value);
        }
        Ok(headers)
    }

    const METADATA_KEYS: [&'static str; 8] = [
        "user_api_key_hash",
        "user_api_key_alias",
        "user_api_key_user_id",
        "user_api_key_user_email",
        "user_api_key_team_id",
        "user_api_key_team_alias",
        "user_api_key_end_user_id",
        "user_api_key_org_id",
    ];

    #[must_use]
    pub fn extract_user_api_key_metadata(
        request_data: &BTreeMap<String, GuardrailValue>,
    ) -> BTreeMap<String, GuardrailValue> {
        let mut merged = BTreeMap::new();
        for key in ["metadata", "litellm_metadata"] {
            if let Some(GuardrailValue::Object(values)) = request_data.get(key) {
                merged.extend(values.clone());
            }
        }
        let mut result = Self::METADATA_KEYS
            .into_iter()
            .filter_map(|key| {
                merged
                    .get(key)
                    .filter(|value| !matches!(value, GuardrailValue::Null))
                    .map(|value| (key.to_owned(), value.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        if !result.contains_key("user_api_key_hash")
            && let Some(value) = merged
                .get("user_api_key_token")
                .filter(|value| !matches!(value, GuardrailValue::Null))
        {
            result.insert("user_api_key_hash".to_owned(), value.clone());
        }
        result
    }

    #[must_use]
    pub fn build_request(&self, inputs: Inputs, context: RequestContext) -> Request {
        let mut provider_params = BTreeMap::from([(
            "firewall_id".to_owned(),
            GuardrailValue::Text(self.settings.deepkeep_firewall_id.clone()),
        )]);
        provider_params.extend(
            context
                .dynamic_params
                .into_iter()
                .filter(|(key, _)| key != "firewall_id"),
        );
        Request {
            litellm_call_id: context.litellm_call_id,
            litellm_trace_id: context.litellm_trace_id,
            texts: inputs.texts,
            images: inputs.images,
            tools: inputs.tools,
            tool_calls: inputs.tool_calls,
            structured_messages: inputs.structured_messages,
            request_data: Self::extract_user_api_key_metadata(&context.request_data),
            litellm_version: context.litellm_version,
            provider_params,
            input_type: context.input_type,
            model: inputs.model,
        }
    }

    pub fn build_return_inputs(
        original: &Inputs,
        response: Response,
    ) -> Result<Inputs, DeepKeepError> {
        if response.action.as_deref() == Some("BLOCKED") {
            return Err(DeepKeepError::Blocked {
                reason: response
                    .blocked_reason
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or_else(|| "Content violates policy".to_owned()),
            });
        }
        Ok(Inputs {
            texts: response.texts.unwrap_or_else(|| original.texts.clone()),
            images: response.images.or_else(|| original.images.clone()),
            tools: response.tools.or_else(|| original.tools.clone()),
            tool_calls: response.tool_calls.or_else(|| original.tool_calls.clone()),
            structured_messages: response
                .structured_messages
                .or_else(|| original.structured_messages.clone()),
            model: original.model.clone(),
        })
    }

    pub async fn apply(
        &self,
        inputs: Inputs,
        context: RequestContext,
    ) -> Result<Inputs, DeepKeepError> {
        let request = self.build_request(inputs.clone(), context);
        let (status, body) = match transport::send_request(self, &request).await {
            Ok(response) => response,
            Err(error) => return self.recover_from_failure(error, inputs),
        };
        if !status.is_success() {
            return self.recover_from_failure(DeepKeepError::Api { status, body }, inputs);
        }
        let parsed_response: Response =
            serde_json::from_str(&body).map_err(|source| DeepKeepError::ResponseJson { source })?;
        Self::build_return_inputs(&inputs, parsed_response)
    }

    fn recover_from_failure(
        &self,
        error: DeepKeepError,
        inputs: Inputs,
    ) -> Result<Inputs, DeepKeepError> {
        let unreachable = match &error {
            DeepKeepError::Request { .. } | DeepKeepError::Timeout { .. } => true,
            DeepKeepError::Api { status, .. } => matches!(
                *status,
                reqwest::StatusCode::BAD_GATEWAY
                    | reqwest::StatusCode::SERVICE_UNAVAILABLE
                    | reqwest::StatusCode::GATEWAY_TIMEOUT
            ),
            DeepKeepError::Blocked { .. }
            | DeepKeepError::ResponseJson { .. }
            | DeepKeepError::RequestBuild { .. }
            | DeepKeepError::InvalidHeader { .. }
            | DeepKeepError::InvalidHeaderName { .. } => false,
        };
        if self.settings.unreachable_fallback == UnreachableFallback::FailOpen && unreachable {
            tracing::warn!(
                guardrail_name = %self.settings.guardrail_name,
                api_base = %self.settings.api_base,
                error = %error,
                "DeepKeep guardrail unreachable; evaluation bypassed because unreachable_fallback is fail_open",
            );
            return Ok(inputs);
        }
        Err(error)
    }
}

#[path = "deepkeep/tests.rs"]
mod tests;
