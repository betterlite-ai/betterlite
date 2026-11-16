use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
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
    MISCONFIGURED_MESSAGE, PROMPT_STAGE, RESPONSE_STAGE, RepelloAILifecycle,
    UNREACHABLE_ERROR_TYPE, UNREACHABLE_MESSAGE,
};

type Detail = BTreeMap<String, GuardrailValue>;

// The text LiteLLM passes to _call_analyze; the stage is its path segment and the event type is the
// hook that asked for the scan
#[derive(Debug, Clone, Copy)]
pub struct Scan<'scan> {
    pub text: &'scan str,
    pub stage: &'static str,
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

    // A failed scan records the detail object, which is what LiteLLM keeps as guardrail_json_response
    fn error(detail: Detail) -> Self {
        Self::failed(http_object(&detail), GuardrailValue::Object(detail))
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
        let run = ScanRun {
            event_type: scan.event_type,
            response: outcome.record,
            status: outcome.status,
            started,
            finished: (self.clock)(),
        };
        record_scan(self, request_data, context, run);
        outcome.result
    }

    // A permanent configuration answer never falls through to the fallback mode.
    async fn analyze(&self, scan: &Scan<'_>) -> ScanOutcome {
        match self.post(scan).await {
            Ok(response) => ScanOutcome::scanned(response),
            Err(PostError::Transient { reason }) => self.unreachable(reason),
            Err(PostError::Config { status }) => ScanOutcome::error(detail(
                MISCONFIGURED_MESSAGE,
                Some(status),
                Some(MISCONFIGURED_ERROR_TYPE),
            )),
            Err(PostError::InvalidJson { status }) => ScanOutcome::error(detail(
                INVALID_JSON_MESSAGE,
                Some(status),
                Some(INVALID_JSON_ERROR_TYPE),
            )),
            Err(PostError::Unexpected { reason }) => ScanOutcome::failed(
                http_object(&detail(FAILED_MESSAGE, None, None)),
                GuardrailValue::Text(reason),
            ),
        }
    }

    fn unreachable(&self, reason: String) -> ScanOutcome {
        tracing::warn!("RepelloAI Argus unreachable: {reason}");
        if self.settings.unreachable_fallback == UnreachableFallback::FailOpen {
            return ScanOutcome::allowed(reason);
        }
        let failure = http_object(&detail(
            UNREACHABLE_MESSAGE,
            None,
            Some(UNREACHABLE_ERROR_TYPE),
        ));
        ScanOutcome::failed(failure, GuardrailValue::Text(reason))
    }

    async fn post(&self, scan: &Scan<'_>) -> Result<RepelloAIAnalyzeResponse, PostError> {
        let endpoint = format!("{}/analyze/{}", self.settings.api_base, scan.stage);
        let scan_data = RepelloAIScanData {
            prompt: (scan.stage == PROMPT_STAGE).then(|| scan.text.to_owned()),
            response: (scan.stage == RESPONSE_STAGE).then(|| scan.text.to_owned()),
        };
        let request = RepelloAIAnalyzeRequest {
            asset_id: self.settings.asset_id.clone(),
            scan_data,
        };
        let mut key =
            HeaderValue::from_str(self.settings.api_key.expose_secret()).map_err(|source| {
                PostError::Transient {
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

fn http_object(detail: &Detail) -> GuardrailFailure {
    GuardrailFailure::HttpObject {
        status: ERROR_STATUS,
        detail: detail
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    }
}

fn detail(message: &str, status: Option<u16>, error_type: Option<&str>) -> Detail {
    let mut detail = Detail::new();
    detail.insert("error".to_owned(), GuardrailValue::Text(message.to_owned()));
    if let Some(code) = status {
        detail.insert(
            "status_code".to_owned(),
            GuardrailValue::UnsignedInteger(u64::from(code)),
        );
    }
    if let Some(kind) = error_type {
        detail.insert(
            "error_type".to_owned(),
            GuardrailValue::Text(kind.to_owned()),
        );
    }
    detail
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

fn recorded_response(response: &RepelloAIAnalyzeResponse) -> GuardrailValue {
    serde_json::to_value(response)
        .and_then(serde_json::from_value)
        .unwrap_or_else(|error| {
            tracing::warn!("RepelloAI Argus response could not be recorded: {error}");
            GuardrailValue::Null
        })
}
