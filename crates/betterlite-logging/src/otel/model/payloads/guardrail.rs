use super::super::utils::coercion::OtelModelValue;
use super::{
    error::SpanError,
    helpers::{guardrail_mode_str, json_or_none, total_masked_entities},
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct GuardrailSpanData {
    pub guardrail_name: String,
    pub mode: Option<String>,
    pub status: Option<String>,
    pub masked_entity_count: Option<i64>,
    pub provider: Option<String>,
    pub action: Option<String>,
    pub response_json: Option<String>,
    pub violation_categories: Vec<String>,
    pub confidence_score: Option<f64>,
    pub risk_score: Option<f64>,
    pub duration: Option<f64>,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    pub guardrail_id: Option<String>,
    pub policy_template: Option<String>,
    pub detection_method: Option<String>,
    pub usage_json: Option<String>,
    pub cost: Option<Decimal>,
    pub cost_in_spend: Option<bool>,
    pub error: Option<SpanError>,
}

impl GuardrailSpanData {
    #[must_use]
    pub fn from_logging_entry(entry: &BTreeMap<String, OtelModelValue>) -> Self {
        let (status, error) = status_error(entry);
        Self {
            guardrail_name: string_field(entry, "guardrail_name")
                .unwrap_or_else(|| "guardrail".to_owned()),
            mode: entry.get("guardrail_mode").and_then(guardrail_mode_str),
            status,
            masked_entity_count: entry
                .get("masked_entity_count")
                .and_then(total_masked_entities),
            provider: string_field(entry, "guardrail_provider"),
            action: string_field(entry, "guardrail_action"),
            response_json: entry.get("guardrail_response").and_then(json_or_none),
            violation_categories: violation_categories(entry),
            confidence_score: float_field(entry, "confidence_score"),
            risk_score: float_field(entry, "risk_score"),
            duration: float_field(entry, "duration"),
            start_time: float_field(entry, "start_time"),
            end_time: float_field(entry, "end_time"),
            guardrail_id: string_field(entry, "guardrail_id"),
            policy_template: string_field(entry, "policy_template"),
            detection_method: string_field(entry, "detection_method"),
            usage_json: entry.get("guardrail_usage").and_then(json_or_none),
            cost: entry
                .get("guardrail_cost")
                .and_then(OtelModelValue::as_decimal),
            cost_in_spend: entry
                .get("guardrail_cost_in_spend")
                .and_then(OtelModelValue::as_bool),
            error,
        }
    }
}

fn string_field(entry: &BTreeMap<String, OtelModelValue>, field: &str) -> Option<String> {
    entry
        .get(field)
        .and_then(OtelModelValue::as_str)
        .map(str::to_owned)
}

fn float_field(entry: &BTreeMap<String, OtelModelValue>, field: &str) -> Option<f64> {
    match entry.get(field) {
        Some(raw) => {
            let decimal = raw.as_decimal()?;
            decimal.to_f64()
        }
        None => None,
    }
}

fn status_error(entry: &BTreeMap<String, OtelModelValue>) -> (Option<String>, Option<SpanError>) {
    let status = string_field(entry, "guardrail_status");
    let error = status
        .as_deref()
        .filter(|status_text| {
            matches!(
                *status_text,
                "guardrail_intervened" | "guardrail_failed_to_respond"
            )
        })
        .map(|status_text| SpanError {
            error_type: Some(status_text.to_owned()),
            message: string_field(entry, "guardrail_action"),
            code: None,
            stack_trace: None,
            llm_provider: None,
        });
    (status, error)
}

fn violation_categories(entry: &BTreeMap<String, OtelModelValue>) -> Vec<String> {
    entry
        .get("violation_categories")
        .and_then(OtelModelValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(OtelModelValue::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[path = "guardrail/tests.rs"]
mod tests;
