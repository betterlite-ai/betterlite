use betterlite_config::sections::grayswan::{Configured, OnFlaggedAction};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::ExposeSecret as _;

use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailInputType, GuardrailMessage, GuardrailToolParam,
};

pub mod error;
mod format;
pub mod wire;

use error::GraySwanGuardrailAPIError;
use format::{build_messages, format_violation_message};
use wire::{
    GraySwanInput, GraySwanPayload, GraySwanResponse, GraySwanResult, GraySwanVerdict,
    GraySwanViolatedRule,
};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct GraySwanGuardrail {
    pub settings: Configured,
    pub client: reqwest::Client,
}
impl GraySwanGuardrail {
    pub fn prepare_headers(&self) -> Result<HeaderMap, GraySwanGuardrailAPIError> {
        let mut headers = HeaderMap::new();
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {}", self.settings.api_key.expose_secret()))
                .map_err(|source| GraySwanGuardrailAPIError::InvalidHeader { source })?;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(headers)
    }
    #[must_use]
    pub fn prepare_payload(
        &self,
        input: &GraySwanInput,
        messages: Vec<GuardrailMessage>,
        tools: Vec<GuardrailToolParam>,
    ) -> GraySwanPayload {
        let categories = input
            .metadata
            .categories
            .clone()
            .filter(|values| !values.is_empty())
            .or_else(|| {
                (!self.settings.categories.is_empty()).then(|| self.settings.categories.clone())
            });
        let policy_id = input
            .metadata
            .policy_id
            .clone()
            .filter(|id| !id.is_empty())
            .or_else(|| {
                (!self.settings.policy_id.is_empty()).then(|| self.settings.policy_id.clone())
            });
        let reasoning_mode = input
            .metadata
            .reasoning_mode
            .or(self.settings.reasoning_mode);
        let metadata = input.metadata.metadata.clone();
        let litellm_metadata = if input.litellm_metadata.is_empty() {
            None
        } else {
            Some(GuardrailExtraValue::Object(input.litellm_metadata.clone()))
        };
        GraySwanPayload {
            messages,
            tools,
            categories,
            policy_id,
            reasoning_mode,
            metadata,
            litellm_metadata,
        }
    }
    pub async fn apply_guardrail(
        &self,
        input: GraySwanInput,
        input_type: GuardrailInputType,
    ) -> Result<GraySwanResult, GraySwanGuardrailAPIError> {
        let tool_calls: &[GuardrailExtraValue] = if input_type == GuardrailInputType::Response {
            &input.tool_calls
        } else {
            &[]
        };
        if input.texts.is_empty() && tool_calls.is_empty() {
            return Ok(GraySwanResult {
                verdict: GraySwanVerdict::Pass,
                texts: input.texts,
                tool_calls: input.tool_calls,
            });
        }
        let context = if input_type == GuardrailInputType::Response {
            input.request_history.clone()
        } else {
            Vec::new()
        };
        let tools = if input_type == GuardrailInputType::Response {
            input.tools.clone()
        } else {
            Vec::new()
        };
        let messages = build_messages(&input.texts, tool_calls, input_type, context);
        let payload = self.prepare_payload(&input, messages, tools);
        match self.call_api(&payload).await {
            Ok(response) => {
                self.process_response(&response, input_type, input.texts, input.tool_calls)
            }
            Err(error) if self.settings.fail_open => {
                tracing::error!(error = %error, "Gray Swan API error; passing through");
                Ok(GraySwanResult {
                    verdict: GraySwanVerdict::Pass,
                    texts: input.texts,
                    tool_calls: input.tool_calls,
                })
            }
            Err(error) => Err(error),
        }
    }
    pub async fn call_api(
        &self,
        payload: &GraySwanPayload,
    ) -> Result<GraySwanResponse, GraySwanGuardrailAPIError> {
        let endpoint = format!(
            "{}/cygnal/monitor",
            self.settings.api_base.trim_end_matches('/')
        );
        let response = self
            .client
            .post(endpoint)
            .headers(self.prepare_headers()?)
            .json(payload)
            .timeout(self.settings.guardrail_timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(GraySwanGuardrailAPIError::HttpStatus { status });
        }
        let body = read_capped_body(response, MAX_RESPONSE_BYTES).await?;
        serde_json::from_slice(&body)
            .map_err(|source| GraySwanGuardrailAPIError::ResponseJson { source })
    }
    pub fn process_response(
        &self,
        response: &GraySwanResponse,
        input_type: GuardrailInputType,
        texts: Vec<String>,
        tool_calls: Vec<GuardrailExtraValue>,
    ) -> Result<GraySwanResult, GraySwanGuardrailAPIError> {
        let flagged = response.violation >= self.settings.violation_threshold;
        if !flagged {
            return Ok(GraySwanResult {
                verdict: GraySwanVerdict::Pass,
                texts,
                tool_calls,
            });
        }
        let is_output = input_type == GuardrailInputType::Response;
        let violation_message = format_violation_message(response, is_output);
        match self.settings.on_flagged_action {
            OnFlaggedAction::Monitor => {
                tracing::info!("Gray Swan Guardrail: Monitoring mode - allowing flagged content");
                Ok(GraySwanResult {
                    verdict: GraySwanVerdict::Monitored { violation_message },
                    texts,
                    tool_calls,
                })
            }
            OnFlaggedAction::Block => Err(blocked_error(response, &violation_message, is_output)),
            OnFlaggedAction::Passthrough if is_output => Ok(GraySwanResult {
                verdict: GraySwanVerdict::Replaced {
                    violation_message: violation_message.clone(),
                },
                texts: vec![violation_message],
                tool_calls,
            }),
            OnFlaggedAction::Passthrough => Ok(GraySwanResult {
                verdict: GraySwanVerdict::ShortCircuited { violation_message },
                texts,
                tool_calls,
            }),
        }
    }
}
fn blocked_error(
    response: &GraySwanResponse,
    violation_message: &str,
    is_output: bool,
) -> GraySwanGuardrailAPIError {
    let violated_rules = response
        .violated_rule_descriptions
        .iter()
        .map(GraySwanViolatedRule::to_extra_value)
        .collect();
    GraySwanGuardrailAPIError::Blocked {
        reason: format!(": {violation_message}"),
        violation_location: if is_output { "output" } else { "input" },
        violation: response.violation,
        violated_rules,
        mutation: response.mutation,
        ipi: response.ipi,
    }
}
fn request_error(source: reqwest::Error) -> GraySwanGuardrailAPIError {
    if source.is_timeout() {
        GraySwanGuardrailAPIError::Timeout { source }
    } else {
        GraySwanGuardrailAPIError::Request { source }
    }
}
async fn read_capped_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, GraySwanGuardrailAPIError> {
    let limit_bytes = u64::try_from(limit).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit_bytes)
    {
        return Err(GraySwanGuardrailAPIError::ResponseTooLarge { limit });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(GraySwanGuardrailAPIError::ResponseTooLarge { limit });
        }
    }
    Ok(body)
}
#[path = "grayswan/tests.rs"]
mod tests;
