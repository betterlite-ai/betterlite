use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
use indexmap::IndexMap;
use reqwest::header::HeaderValue;
use secrecy::ExposeSecret as _;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_records::record::GuardrailStatus;
use crate::repello_content::wire::{
    RepelloAIAnalyzeRequest, RepelloAIAnalyzeResponse, RepelloAIScanData,
};

use super::hooks::HookContext;
use super::records::{ScanRun, record_scan};
use super::verdict::verdict_blocks;
use super::{
    API_KEY_HEADER, CONFIG_ERROR_STATUS_CODES, Data, ERROR_STATUS, FAILED_MESSAGE,
    INVALID_JSON_ERROR_TYPE, INVALID_JSON_MESSAGE, MAX_RESPONSE_BYTES, MISCONFIGURED_ERROR_TYPE,
    MISCONFIGURED_MESSAGE, RepelloAILifecycle, UNREACHABLE_ERROR_TYPE, UNREACHABLE_MESSAGE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepelloStage {
    Prompt,
    Response,
}

impl RepelloStage {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Response => "response",
        }
    }
}

// The text LiteLLM passes to _call_analyze; the event type is the hook that asked for the scan
#[derive(Debug, Clone, Copy)]
pub struct Scan<'scan> {
    pub text: &'scan str,
    pub stage: RepelloStage,
    pub event_type: GuardrailEventHooks,
}

enum PostError {
    Config { status: u16 },
    InvalidJson { status: u16 },
    Transient { reason: String },
    Unexpected { reason: String },
}

struct ScanOutcome {
    result: Result<Option<RepelloAIAnalyzeResponse>, GuardrailFailure>,
    record: GuardrailValue,
    status: GuardrailStatus,
}

impl ScanOutcome {
    fn scanned(response: RepelloAIAnalyzeResponse) -> Self {
        let status = if verdict_blocks(Some(&response)) {
            GuardrailStatus::GuardrailIntervened
        } else {
            GuardrailStatus::Success
        };
        Self {
            record: recorded_response(&response),
            status,
            result: Ok(Some(response)),
        }
    }

    fn allowed(reason: String) -> Self {
        Self {
            result: Ok(None),
            record: GuardrailValue::Text(reason),
            status: GuardrailStatus::GuardrailFailedToRespond,
        }
    }

    fn failed(failure: GuardrailFailure, record: GuardrailValue) -> Self {
        Self {
            result: Err(failure),
            record,
            status: GuardrailStatus::GuardrailFailedToRespond,
        }
    }
}

impl RepelloAILifecycle {
    // LiteLLM's _call_analyze: the scan is recorded in a finally, so every outcome writes one record.
    pub async fn call_analyze(
        &self,
        scan: Scan<'_>,
        request_data: &mut Data,
        context: &mut HookContext<'_>,
    ) -> Result<Option<RepelloAIAnalyzeResponse>, GuardrailFailure> {
        let started = (self.clock)();
        let outcome = self.analyze(&scan).await;
        let finished = (self.clock)();
        let run = ScanRun {
            event_type: scan.event_type,
            response: outcome.record,
            status: outcome.status,
            started,
            finished,
        };
        record_scan(self, request_data, context, run);
        outcome.result
    }

    async fn analyze(&self, scan: &Scan<'_>) -> ScanOutcome {
        match self.post(scan).await {
            Ok(response) => ScanOutcome::scanned(response),
            Err(PostError::Transient { reason }) => self.unreachable(reason),
            Err(error) => failure_outcome(error),
        }
    }

    fn unreachable(&self, reason: String) -> ScanOutcome {
        tracing::warn!("RepelloAI Argus unreachable: {reason}");
        if self.settings.unreachable_fallback == UnreachableFallback::FailOpen {
            return ScanOutcome::allowed(reason);
        }
        http_object_outcome(unreachable_detail(), GuardrailValue::Text(reason))
    }

    async fn post(&self, scan: &Scan<'_>) -> Result<RepelloAIAnalyzeResponse, PostError> {
        let endpoint = format!("{}/analyze/{}", self.settings.api_base, scan.stage.as_str());
        let scan_data = match scan.stage {
            RepelloStage::Prompt => RepelloAIScanData {
                prompt: Some(scan.text.to_owned()),
                response: None,
            },
            RepelloStage::Response => RepelloAIScanData {
                prompt: None,
                response: Some(scan.text.to_owned()),
            },
        };
        let request = RepelloAIAnalyzeRequest {
            asset_id: self.settings.asset_id.clone(),
            scan_data,
        };
        let mut key =
            HeaderValue::from_str(self.settings.api_key.expose_secret()).map_err(|source| {
                PostError::Unexpected {
                    reason: source.to_string(),
                }
            })?;
        key.set_sensitive(true);
        let mut response = self
            .client
            .post(endpoint)
            .header(API_KEY_HEADER, key)
            .json(&request)
            .timeout(self.settings.request_timeout)
            .send()
            .await
            .map_err(send_error)?;
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
        serde_json::from_slice(&body).map_err(|_decode| PostError::InvalidJson { status: code })
    }
}

// A permanent configuration answer never falls through to the fallback mode.
fn failure_outcome(error: PostError) -> ScanOutcome {
    let detail = match &error {
        PostError::Config { status } => misconfigured_detail(*status),
        PostError::InvalidJson { status } => invalid_json_detail(*status),
        PostError::Transient { .. } | PostError::Unexpected { .. } => error_detail(FAILED_MESSAGE),
    };
    let record = match error {
        PostError::Unexpected { reason } => GuardrailValue::Text(reason),
        PostError::Config { .. } | PostError::InvalidJson { .. } | PostError::Transient { .. } => {
            detail_record(&detail)
        }
    };
    http_object_outcome(detail, record)
}

fn http_object_outcome(
    detail: IndexMap<String, GuardrailValue>,
    record: GuardrailValue,
) -> ScanOutcome {
    ScanOutcome::failed(
        GuardrailFailure::HttpObject {
            status: ERROR_STATUS,
            detail,
        },
        record,
    )
}

// A transport error, a timeout included, is httpx's HTTPError, which LiteLLM sends to _handle_unreachable.
fn send_error(error: reqwest::Error) -> PostError {
    PostError::Transient {
        reason: error.to_string(),
    }
}

fn read_error(error: ReadError) -> PostError {
    match error {
        ReadError::Failed(source) => send_error(source),
        ReadError::TooLarge { limit } => PostError::Unexpected {
            reason: format!("the RepelloAI Argus response body is larger than {limit} bytes"),
        },
    }
}

fn error_detail(message: &str) -> IndexMap<String, GuardrailValue> {
    let mut detail = IndexMap::new();
    detail.insert("error".to_owned(), GuardrailValue::Text(message.to_owned()));
    detail
}

fn misconfigured_detail(status: u16) -> IndexMap<String, GuardrailValue> {
    let mut detail = error_detail(MISCONFIGURED_MESSAGE);
    detail.insert(
        "status_code".to_owned(),
        GuardrailValue::UnsignedInteger(u64::from(status)),
    );
    detail.insert(
        "error_type".to_owned(),
        GuardrailValue::Text(MISCONFIGURED_ERROR_TYPE.to_owned()),
    );
    detail
}

fn invalid_json_detail(status: u16) -> IndexMap<String, GuardrailValue> {
    let mut detail = error_detail(INVALID_JSON_MESSAGE);
    detail.insert(
        "status_code".to_owned(),
        GuardrailValue::UnsignedInteger(u64::from(status)),
    );
    detail.insert(
        "error_type".to_owned(),
        GuardrailValue::Text(INVALID_JSON_ERROR_TYPE.to_owned()),
    );
    detail
}

fn unreachable_detail() -> IndexMap<String, GuardrailValue> {
    let mut detail = error_detail(UNREACHABLE_MESSAGE);
    detail.insert(
        "error_type".to_owned(),
        GuardrailValue::Text(UNREACHABLE_ERROR_TYPE.to_owned()),
    );
    detail
}

fn detail_record(detail: &IndexMap<String, GuardrailValue>) -> GuardrailValue {
    let mut object = BTreeMap::new();
    for (key, value) in detail {
        object.insert(key.clone(), value.clone());
    }
    GuardrailValue::Object(object)
}

fn recorded_response(response: &RepelloAIAnalyzeResponse) -> GuardrailValue {
    serde_json::to_value(response)
        .and_then(serde_json::from_value)
        .unwrap_or_else(|error| {
            tracing::warn!("RepelloAI Argus response could not be recorded: {error}");
            GuardrailValue::Null
        })
}
