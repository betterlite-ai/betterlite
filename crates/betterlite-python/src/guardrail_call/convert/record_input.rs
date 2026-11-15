use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailRecordInput, GuardrailStatus, GuardrailTracingDetail,
};
use rust_decimal::Decimal;
use serde::Deserialize;

use super::exact_value;
use crate::guardrail_call::error::CallError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordInputJson {
    guardrail_response: GuardrailValue,
    guardrail_status: GuardrailStatus,
    #[serde(default)]
    start_time: Option<f64>,
    #[serde(default)]
    end_time: Option<f64>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    masked_entity_count: Option<BTreeMap<String, i64>>,
    #[serde(default)]
    guardrail_provider: Option<String>,
    #[serde(default)]
    event_type: Option<GuardrailEventHooks>,
    #[serde(default)]
    tracing_detail: Option<TracingDetailJson>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TracingDetailJson {
    guardrail_id: Option<String>,
    policy_template: Option<String>,
    detection_method: Option<String>,
    confidence_score: Option<f64>,
    classification: Option<GuardrailValue>,
    match_details: Option<GuardrailValue>,
    patterns_checked: Option<i64>,
    alert_recipients: Option<Vec<String>>,
    risk_score: Option<f64>,
    violation_categories: Option<Vec<String>>,
    guardrail_action: Option<String>,
    guardrail_usage: Option<BTreeMap<String, i64>>,
    guardrail_cost: Option<Decimal>,
    guardrail_cost_by_unit: Option<BTreeMap<String, Option<Decimal>>>,
    guardrail_cost_in_spend: Option<bool>,
}

impl TracingDetailJson {
    fn into_detail(self) -> Result<GuardrailTracingDetail, CallError> {
        Ok(GuardrailTracingDetail {
            guardrail_id: self.guardrail_id,
            policy_template: self.policy_template,
            detection_method: self.detection_method,
            confidence_score: self.confidence_score,
            classification: self.classification.map(exact_value).transpose()?,
            match_details: self.match_details.map(exact_value).transpose()?,
            patterns_checked: self.patterns_checked,
            alert_recipients: self.alert_recipients,
            risk_score: self.risk_score,
            violation_categories: self.violation_categories,
            guardrail_action: self.guardrail_action,
            guardrail_usage: self.guardrail_usage,
            guardrail_cost: self.guardrail_cost,
            guardrail_cost_by_unit: self.guardrail_cost_by_unit,
            guardrail_cost_in_spend: self.guardrail_cost_in_spend,
        })
    }
}

pub(crate) fn record_input_from_text(text: &str) -> Result<GuardrailRecordInput, CallError> {
    let raw: RecordInputJson = serde_json::from_str(text).map_err(|source| CallError::Json {
        field: "guardrail record input",
        source,
    })?;
    Ok(GuardrailRecordInput {
        guardrail_response: exact_value(raw.guardrail_response)?,
        guardrail_status: raw.guardrail_status,
        start_time: raw.start_time,
        end_time: raw.end_time,
        duration: raw.duration,
        masked_entity_count: raw.masked_entity_count,
        guardrail_provider: raw.guardrail_provider,
        event_type: raw.event_type,
        tracing_detail: raw
            .tracing_detail
            .map(TracingDetailJson::into_detail)
            .transpose()?,
    })
}
