use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::ContentFilterDetection;
use serde::Serialize;

use super::detection_metrics::{MatchDetail, build_match_details, get_detection_methods};
use super::risk_scoring::{GuardrailStatus, compute_risk_score, elapsed_seconds};

mod tests;

#[derive(Debug, Clone, Serialize)]
pub struct GuardrailTracingDetail {
    pub guardrail_id: String,
    pub policy_template: Option<String>,
    pub detection_method: Option<String>,
    pub match_details: Option<Vec<MatchDetail>>,
    pub patterns_checked: usize,
    pub risk_score: f64,
    pub confidence_score: Option<f64>,
    pub classification: Option<ContentFilterDetection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuardrailLogRecord {
    pub guardrail_name: String,
    pub guardrail_provider: String,
    pub guardrail_response: String,
    pub guardrail_status: GuardrailStatus,
    pub start_time: f64,
    pub end_time: f64,
    pub duration: f64,
    pub masked_entity_count: BTreeMap<String, usize>,
    pub tracing_detail: GuardrailTracingDetail,
}

pub struct GuardrailTraceInput<'input> {
    pub guardrail_provider: &'input str,
    pub guardrail_name: &'input str,
    pub guardrail_id: Option<&'input str>,
    pub policy_template: Option<String>,
    pub detections: &'input [ContentFilterDetection],
    pub status: GuardrailStatus,
    pub start_time: f64,
    pub end_time: f64,
    pub duration: Option<f64>,
    pub masked_entity_count: &'input BTreeMap<String, usize>,
    pub patterns_checked: usize,
    pub exception_str: Option<&'input str>,
}

pub fn log_guardrail_information(
    records: &mut Vec<GuardrailLogRecord>,
    input: &GuardrailTraceInput<'_>,
) -> GuardrailTracingDetail {
    let tracing_detail = tracing_detail(input);
    let guardrail_response = if input.status == GuardrailStatus::Success {
        format!("{:?}", input.detections)
    } else if let Some(exception) = input.exception_str {
        exception.to_owned()
    } else {
        format!("{:?}", input.detections)
    };
    records.push(GuardrailLogRecord {
        guardrail_name: input.guardrail_name.to_owned(),
        guardrail_provider: input.guardrail_provider.to_owned(),
        guardrail_response,
        guardrail_status: input.status,
        start_time: input.start_time,
        end_time: input.end_time,
        duration: input
            .duration
            .unwrap_or_else(|| elapsed_seconds(input.end_time, input.start_time)),
        masked_entity_count: input.masked_entity_count.clone(),
        tracing_detail: tracing_detail.clone(),
    });
    tracing_detail
}

fn tracing_detail(input: &GuardrailTraceInput<'_>) -> GuardrailTracingDetail {
    let competitor = input
        .detections
        .iter()
        .find_map(|detection| match detection {
            ContentFilterDetection::CompetitorIntent(intent) => Some(intent),
            ContentFilterDetection::Pattern(_)
            | ContentFilterDetection::BlockedWord(_)
            | ContentFilterDetection::CategoryKeyword(_) => None,
        });
    GuardrailTracingDetail {
        guardrail_id: input
            .guardrail_id
            .unwrap_or(input.guardrail_name)
            .to_owned(),
        policy_template: input.policy_template.clone(),
        detection_method: (!input.detections.is_empty())
            .then(|| get_detection_methods(input.detections)),
        match_details: (!input.detections.is_empty())
            .then(|| build_match_details(input.detections)),
        patterns_checked: input.patterns_checked,
        risk_score: compute_risk_score(
            input.detections,
            input.masked_entity_count,
            input.patterns_checked,
            input.status,
        ),
        confidence_score: competitor.map(|intent| intent.confidence),
        classification: competitor
            .map(|intent| ContentFilterDetection::CompetitorIntent(intent.clone())),
    }
}
