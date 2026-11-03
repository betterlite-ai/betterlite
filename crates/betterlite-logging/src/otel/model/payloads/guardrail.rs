use super::{
    error::SpanError,
    helpers::{guardrail_mode_str, json_or_none, total_masked_entities},
};
use rust_decimal::Decimal;
use serde_json::Value;
use std::collections::BTreeMap;
use std::str::FromStr as _;

#[derive(Clone, Debug, Default, PartialEq)]
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
    pub fn from_logging_entry(entry: &BTreeMap<String, Value>) -> Self {
        let text = |name: &str| entry.get(name).and_then(Value::as_str).map(str::to_owned);
        let status = text("guardrail_status");
        let error = status
            .as_deref()
            .filter(|status| {
                matches!(
                    *status,
                    "guardrail_intervened" | "guardrail_failed_to_respond"
                )
            })
            .map(|status| SpanError {
                error_type: Some(status.to_owned()),
                message: text("guardrail_action"),
                code: None,
                stack_trace: None,
                llm_provider: None,
            });
        Self {
            guardrail_name: text("guardrail_name").unwrap_or_else(|| "guardrail".to_owned()),
            mode: entry.get("guardrail_mode").and_then(guardrail_mode_str),
            status,
            masked_entity_count: entry
                .get("masked_entity_count")
                .and_then(total_masked_entities),
            provider: text("guardrail_provider"),
            action: text("guardrail_action"),
            response_json: entry.get("guardrail_response").and_then(json_or_none),
            violation_categories: entry
                .get("violation_categories")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            confidence_score: entry.get("confidence_score").and_then(Value::as_f64),
            risk_score: entry.get("risk_score").and_then(Value::as_f64),
            duration: entry.get("duration").and_then(Value::as_f64),
            start_time: entry.get("start_time").and_then(Value::as_f64),
            end_time: entry.get("end_time").and_then(Value::as_f64),
            guardrail_id: text("guardrail_id"),
            policy_template: text("policy_template"),
            detection_method: text("detection_method"),
            usage_json: entry.get("guardrail_usage").and_then(json_or_none),
            cost: entry
                .get("guardrail_cost")
                .and_then(|value| Decimal::from_str(&value.to_string()).ok()),
            cost_in_spend: entry
                .get("guardrail_cost_in_spend")
                .and_then(Value::as_bool),
            error,
        }
    }
}

#[path = "guardrail/tests.rs"]
mod tests;
