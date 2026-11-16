use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
use reqwest::header::HeaderValue;
use secrecy::ExposeSecret as _;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_records::record::{
    GuardrailRecordContext, GuardrailRecordInput, GuardrailStatus,
};
use crate::repello_content::wire::{
    RepelloAIAnalyzeRequest, RepelloAIAnalyzeResponse, RepelloAIScanData,
};

use super::answer::Answer;
use super::hooks::HookContext;
use super::verdict::verdict_blocks;
use super::{
    API_KEY_HEADER, CONFIG_ERROR_STATUS_CODES, Data, ERROR_STATUS, FAILED_MESSAGE,
    INVALID_JSON_ERROR_TYPE, INVALID_JSON_MESSAGE, MAX_RESPONSE_BYTES, MISCONFIGURED_ERROR_TYPE,
    MISCONFIGURED_MESSAGE, PROMPT_STAGE, RESPONSE_STAGE, RepelloAILifecycle,
    UNREACHABLE_ERROR_TYPE, UNREACHABLE_MESSAGE,
};

type Detail = BTreeMap<String, GuardrailValue>;
// The scan's result, its record and its logging status: what LiteLLM writes in _call_analyze's finally.
type Outcome = (
    Result<Option<RepelloAIAnalyzeResponse>, GuardrailFailure>,
    GuardrailValue,
    GuardrailStatus,
);

enum PostError {
    Config { status: u16 },
    InvalidJson { status: u16 },
    Transient { reason: String },
    Unexpected { reason: String },
}

impl RepelloAILifecycle {
    // LiteLLM's _call_analyze: the scan is recorded in a finally, so every outcome writes one record.
    pub async fn call_analyze(
        &self,
        text: &str,
        stage: &'static str,
        request_data: &mut Data,
        context: &mut HookContext<'_>,
    ) -> Result<Option<RepelloAIAnalyzeResponse>, GuardrailFailure> {
        // LiteLLM scans the prompt in the pre_call hook and the response in both post_call hooks.
        let event_type = if stage == PROMPT_STAGE {
            GuardrailEventHooks::PreCall
        } else {
            GuardrailEventHooks::PostCall
        };
        let started = (self.clock)();
        let (result, response, status) = self.analyze(text, stage).await;
        let finished = (self.clock)();
        let duration = finished.duration_since(started).unwrap_or_default();
        let input = GuardrailRecordInput {
            guardrail_response: response,
            guardrail_status: status,
            start_time: Some(epoch(started)),
            end_time: Some(epoch(finished)),
            duration: Some(duration.as_secs_f64()),
            masked_entity_count: Some(BTreeMap::new()),
            guardrail_provider: None,
            event_type: Some(event_type),
            tracing_detail: None,
        };
        let mut record = GuardrailRecordContext {
            base: &self.base,
            request_data,
            log_target: context.log_target.as_deref_mut(),
            sink: context.sink,
        };
        if let Err(error) = context.invocation.record_own(&mut record, input) {
            tracing::warn!("RepelloAI Argus guardrail record was not written: {error}");
        }
        result
    }

    async fn analyze(&self, text: &str, stage: &'static str) -> Outcome {
        match self.post(text, stage).await {
            Ok(response) => {
                let status = if verdict_blocks(Some(&response)) {
                    GuardrailStatus::GuardrailIntervened
                } else {
                    GuardrailStatus::Success
                };
                let record = serde_json::to_value(&response)
                    .and_then(serde_json::from_value)
                    .unwrap_or(GuardrailValue::Null);
                (Ok(Some(response)), record, status)
            }
            Err(PostError::Transient { reason }) => self.unreachable(reason),
            Err(PostError::Config { status }) => failed(detail(
                MISCONFIGURED_MESSAGE,
                Some(status),
                Some(MISCONFIGURED_ERROR_TYPE),
            )),
            Err(PostError::InvalidJson { status }) => failed(detail(
                INVALID_JSON_MESSAGE,
                Some(status),
                Some(INVALID_JSON_ERROR_TYPE),
            )),
            Err(PostError::Unexpected { reason }) => (
                Err(http_object(detail(FAILED_MESSAGE, None, None))),
                GuardrailValue::Text(reason),
                GuardrailStatus::GuardrailFailedToRespond,
            ),
        }
    }

    fn unreachable(&self, reason: String) -> Outcome {
        tracing::warn!("RepelloAI Argus unreachable: {reason}");
        let result = if self.settings.unreachable_fallback == UnreachableFallback::FailOpen {
            Ok(None)
        } else {
            let failure = detail(UNREACHABLE_MESSAGE, None, Some(UNREACHABLE_ERROR_TYPE));
            Err(http_object(failure))
        };
        (
            result,
            GuardrailValue::Text(reason),
            GuardrailStatus::GuardrailFailedToRespond,
        )
    }

    async fn post(
        &self,
        text: &str,
        stage: &'static str,
    ) -> Result<RepelloAIAnalyzeResponse, PostError> {
        let request = RepelloAIAnalyzeRequest {
            asset_id: self.settings.asset_id.clone(),
            scan_data: RepelloAIScanData {
                prompt: (stage == PROMPT_STAGE).then(|| text.to_owned()),
                response: (stage == RESPONSE_STAGE).then(|| text.to_owned()),
            },
        };
        let mut key =
            HeaderValue::from_str(self.settings.api_key.expose_secret()).map_err(|source| {
                PostError::Unexpected {
                    reason: source.to_string(),
                }
            })?;
        key.set_sensitive(true);
        let response = self
            .client
            .post(format!("{}/analyze/{stage}", self.settings.api_base))
            .header(API_KEY_HEADER, key)
            .json(&request)
            .timeout(self.settings.request_timeout)
            .send()
            .await
            .map_err(|error| PostError::Transient {
                reason: error.to_string(),
            })?;
        decode(response).await
    }
}

// LiteLLM's _raise_for_config_error runs before the status check, so a permanent answer is never transient.
async fn decode(mut response: reqwest::Response) -> Result<RepelloAIAnalyzeResponse, PostError> {
    let status = response.status();
    let code = status.as_u16();
    if CONFIG_ERROR_STATUS_CODES.contains(&code) {
        return Err(PostError::Config { status: code });
    }
    if !status.is_success() {
        return Err(PostError::Transient {
            reason: format!("the RepelloAI Argus API answered {status}"),
        });
    }
    let body = read_bounded(&mut response, MAX_RESPONSE_BYTES)
        .await
        .map_err(read_error)?;
    // LiteLLM's TypeAdapter(RepelloAIAnalyzeResponse).validate_json: the answer the Argus API sent, typed.
    serde_json::from_slice::<Answer>(&body)
        .map(RepelloAIAnalyzeResponse::from)
        .map_err(|_malformed| PostError::InvalidJson { status: code })
}

// A failed scan records the detail object, which is what LiteLLM keeps as guardrail_json_response.
fn failed(detail: Detail) -> Outcome {
    (
        Err(http_object(detail.clone())),
        GuardrailValue::Object(detail),
        GuardrailStatus::GuardrailFailedToRespond,
    )
}

fn http_object(detail: Detail) -> GuardrailFailure {
    GuardrailFailure::HttpObject {
        status: ERROR_STATUS,
        detail: detail.into_iter().collect(),
    }
}

fn detail(message: &str, status: Option<u16>, error_type: Option<&str>) -> Detail {
    let mut detail = Detail::from([("error".to_owned(), GuardrailValue::Text(message.to_owned()))]);
    detail.extend(status.map(|code| {
        let number = GuardrailValue::UnsignedInteger(u64::from(code));
        ("status_code".to_owned(), number)
    }));
    detail.extend(error_type.map(|kind| {
        (
            "error_type".to_owned(),
            GuardrailValue::Text(kind.to_owned()),
        )
    }));
    detail
}

fn read_error(error: ReadError) -> PostError {
    match error {
        ReadError::Failed(source) => PostError::Transient {
            reason: source.to_string(),
        },
        ReadError::TooLarge { limit } => PostError::Unexpected {
            reason: format!("the RepelloAI Argus response body is larger than {limit} bytes"),
        },
    }
}

fn epoch(instant: SystemTime) -> f64 {
    instant
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
