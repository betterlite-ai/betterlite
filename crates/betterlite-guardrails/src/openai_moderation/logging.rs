use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use super::error::ModerationError;
use super::wire::ModerationResponse;
use crate::content_filter::risk_scoring::GuardrailStatus;

const ALLOW: &str = "allow";

#[derive(Debug, Clone, Copy)]
pub struct CallTiming {
    pub started: Duration,
    pub ended: Duration,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GuardrailResponse {
    Moderation(ModerationResponse),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TracingDetail {
    pub violation_categories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardrailLogRecord {
    pub guardrail_name: String,
    pub guardrail_response: GuardrailResponse,
    pub guardrail_status: GuardrailStatus,
    pub start_time: f64,
    pub end_time: f64,
    pub duration: f64,
    #[serde(flatten)]
    pub tracing_detail: Option<TracingDetail>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RequestMetadata {
    #[serde(
        rename = "_openai_moderation_response",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub openai_moderation_response: Option<ModerationResponse>,
    #[serde(
        rename = "standard_logging_guardrail_information",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub guardrail_records: Vec<GuardrailLogRecord>,
    #[serde(flatten)]
    pub rest: BTreeMap<String, GuardrailValue>,
}

// LiteLLM treats 400, 403 and 422 as deliberate blocks and every other failure as no answer.
const INTERVENTION_STATUSES: [StatusCode; 3] = [
    StatusCode::BAD_REQUEST,
    StatusCode::FORBIDDEN,
    StatusCode::UNPROCESSABLE_ENTITY,
];

#[must_use]
pub fn build_tracing_detail(guardrail_response: &GuardrailResponse) -> Option<TracingDetail> {
    let GuardrailResponse::Moderation(moderation_response) = guardrail_response else {
        return None;
    };
    let violation_categories: Vec<String> = moderation_response
        .results
        .iter()
        .flat_map(|result| result.categories.iter().flatten())
        .filter(|(_, is_flagged)| **is_flagged)
        .map(|(category, _)| category.clone())
        .collect();
    (!violation_categories.is_empty()).then_some(TracingDetail {
        violation_categories,
    })
}

pub fn process_response(guardrail_name: &str, metadata: &mut RequestMetadata, timing: CallTiming) {
    let guardrail_response = metadata.openai_moderation_response.take().map_or_else(
        || GuardrailResponse::Text(ALLOW.to_owned()),
        GuardrailResponse::Moderation,
    );
    add_log_record(
        guardrail_name,
        metadata,
        (guardrail_response, GuardrailStatus::Success),
        timing,
    );
}

pub fn process_error(
    guardrail_name: &str,
    error: &ModerationError,
    metadata: &mut RequestMetadata,
    timing: CallTiming,
) {
    let status = if INTERVENTION_STATUSES.contains(&error.status_code()) {
        GuardrailStatus::GuardrailIntervened
    } else {
        GuardrailStatus::GuardrailFailedToRespond
    };
    let guardrail_response = metadata.openai_moderation_response.take().map_or_else(
        || GuardrailResponse::Text(error.to_string()),
        GuardrailResponse::Moderation,
    );
    add_log_record(
        guardrail_name,
        metadata,
        (guardrail_response, status),
        timing,
    );
}

fn add_log_record(
    guardrail_name: &str,
    metadata: &mut RequestMetadata,
    (guardrail_response, guardrail_status): (GuardrailResponse, GuardrailStatus),
    timing: CallTiming,
) {
    let tracing_detail = build_tracing_detail(&guardrail_response);
    metadata.guardrail_records.push(GuardrailLogRecord {
        guardrail_name: guardrail_name.to_owned(),
        guardrail_response,
        guardrail_status,
        start_time: timing.started.as_secs_f64(),
        end_time: timing.ended.as_secs_f64(),
        duration: timing.ended.saturating_sub(timing.started).as_secs_f64(),
        tracing_detail,
    });
}
