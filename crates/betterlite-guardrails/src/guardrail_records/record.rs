use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::ser::Error as SerializeError;
use serde::{Serialize, Serializer};

use super::error::GuardrailSpanError;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::inputs::GuardrailLogTarget;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailStatus {
    Success,
    GuardrailFlagged,
    GuardrailIntervened,
    GuardrailFailedToRespond,
    NotRun,
}

fn cost_number<E>(amount: &Decimal) -> Result<f64, E>
where
    E: SerializeError,
{
    amount
        .to_f64()
        .ok_or_else(|| E::custom(format!("cost {amount} is not a finite number")))
}

// serde hands the field as &Option, and a pub fn keeps clippy's ref_option quiet
pub fn serialize_cost<S>(cost: &Option<Decimal>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match cost {
        Some(amount) => serializer.serialize_f64(cost_number(amount)?),
        None => serializer.serialize_none(),
    }
}

pub fn serialize_costs_by_unit<S>(
    unit_costs: &Option<BTreeMap<String, Option<Decimal>>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let Some(costs) = unit_costs else {
        return serializer.serialize_none();
    };
    let mut numbers = BTreeMap::new();
    for (unit, cost) in costs {
        let number = cost.as_ref().map(cost_number::<S::Error>).transpose()?;
        numbers.insert(unit, number);
    }
    numbers.serialize(serializer)
}

// LiteLLM's GuardrailTracingDetail; an unset field is left out of the record
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GuardrailTracingDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guardrail_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detection_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<GuardrailValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_details: Option<GuardrailValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patterns_checked: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert_recipients: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub violation_categories: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guardrail_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guardrail_usage: Option<BTreeMap<String, i64>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_cost"
    )]
    pub guardrail_cost: Option<Decimal>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_costs_by_unit"
    )]
    pub guardrail_cost_by_unit: Option<BTreeMap<String, Option<Decimal>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guardrail_cost_in_spend: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum GuardrailRecordMode {
    Event(GuardrailEventHooks),
    Configured(LitellmParamsMode),
}

// LiteLLM's StandardLoggingGuardrailInformation; the nine base keys are always written
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuardrailInvocationRecord {
    pub guardrail_name: Option<String>,
    pub guardrail_provider: Option<String>,
    pub guardrail_mode: Option<GuardrailRecordMode>,
    pub guardrail_response: GuardrailValue,
    pub guardrail_status: GuardrailStatus,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    pub duration: Option<f64>,
    pub masked_entity_count: Option<BTreeMap<String, i64>>,
    #[serde(flatten)]
    pub tracing_detail: GuardrailTracingDetail,
}

// The arguments of add_standard_logging_guardrail_information_to_request_data
#[derive(Debug, Clone, PartialEq)]
pub struct GuardrailRecordInput {
    pub guardrail_response: GuardrailValue,
    pub guardrail_status: GuardrailStatus,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    pub duration: Option<f64>,
    pub masked_entity_count: Option<BTreeMap<String, i64>>,
    pub guardrail_provider: Option<String>,
    pub event_type: Option<GuardrailEventHooks>,
    pub tracing_detail: Option<GuardrailTracingDetail>,
}

pub struct GuardrailRecordContext<'context> {
    pub base: &'context GuardrailBase,
    pub request_data: &'context mut BTreeMap<String, GuardrailValue>,
    pub log_target: Option<&'context mut GuardrailLogTarget>,
    pub sink: &'context dyn GuardrailSpanSink,
}

pub trait GuardrailSpanSink {
    fn emit(&self, record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError>;
}
