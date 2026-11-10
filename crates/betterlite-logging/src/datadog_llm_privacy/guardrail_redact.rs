use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::json_number::number as json_number;
use crate::datadog_llm_obs::constants::{AUDIT_GUARDRAIL_FIELDS, GUARDRAIL_PROMPT_FIELDS};
use crate::datadog_llm_obs::values::{Object, as_object};
use crate::fields::payload::records::{DecimalValue, GuardrailInformation};
use crate::spend_sanitize::REDACTED_BY_LITELM;

// LiteLLM keeps the audit fields and marks the prompt-carrying ones; a key in neither set is dropped
#[must_use]
pub fn redact_guardrails(value: Option<&Json>) -> Option<Vec<Object>> {
    let records = value?;
    let redacted: Vec<Object> = entries(records).into_iter().map(redacted_record).collect();
    Some(redacted)
}

#[must_use]
pub fn redacted_records(records: &[GuardrailInformation]) -> Vec<Object> {
    records
        .iter()
        .map(|record| redacted_record(&guardrail_record(record)))
        .collect()
}

#[must_use]
pub fn guardrail_record(record: &GuardrailInformation) -> Object {
    let mut object = Object::new();
    for (field, value) in identity_fields(record)
        .into_iter()
        .chain(timing_fields(record))
        .chain(cost_fields(record))
        .chain(prompt_fields(record))
    {
        if let Some(field_value) = value {
            object.insert(field.to_owned(), field_value);
        }
    }
    object
}

fn identity_fields(record: &GuardrailInformation) -> [(&'static str, Option<Json>); 8] {
    let text = |value: &String| Json::Text(value.clone());
    let content = |value: &Json| value.clone();
    [
        (
            "guardrail_name",
            optional(record.guardrail_name.as_ref(), text),
        ),
        (
            "guardrail_provider",
            optional(record.guardrail_provider.as_ref(), text),
        ),
        (
            "guardrail_status",
            optional(record.guardrail_status.as_ref(), text),
        ),
        ("guardrail_id", optional(record.guardrail_id.as_ref(), text)),
        (
            "policy_template",
            optional(record.policy_template.as_ref(), text),
        ),
        (
            "detection_method",
            optional(record.detection_method.as_ref(), text),
        ),
        (
            "guardrail_action",
            optional(record.guardrail_action.as_ref(), text),
        ),
        (
            "guardrail_mode",
            optional(record.guardrail_mode.as_ref(), content),
        ),
    ]
}

fn timing_fields(record: &GuardrailInformation) -> [(&'static str, Option<Json>); 7] {
    let money = |value: &DecimalValue| json_number(value.0);
    let whole = |value: &i64| json_number(Decimal::from(*value));
    [
        ("start_time", optional(record.start_time.as_ref(), money)),
        ("end_time", optional(record.end_time.as_ref(), money)),
        ("duration", optional(record.duration.as_ref(), money)),
        (
            "masked_entity_count",
            optional(record.masked_entity_count.as_ref(), counted),
        ),
        (
            "confidence_score",
            optional(record.confidence_score.as_ref(), money),
        ),
        (
            "patterns_checked",
            optional(record.patterns_checked.as_ref(), whole),
        ),
        ("risk_score", optional(record.risk_score.as_ref(), money)),
    ]
}

fn cost_fields(record: &GuardrailInformation) -> [(&'static str, Option<Json>); 6] {
    let money = |value: &DecimalValue| json_number(value.0);
    [
        (
            "alert_recipients",
            optional(record.alert_recipients.as_ref(), |items| listed(items)),
        ),
        (
            "violation_categories",
            optional(record.violation_categories.as_ref(), |items| listed(items)),
        ),
        (
            "guardrail_usage",
            optional(record.guardrail_usage.as_ref(), counted),
        ),
        (
            "guardrail_cost",
            optional(record.guardrail_cost.as_ref(), money),
        ),
        (
            "guardrail_cost_by_unit",
            optional(record.guardrail_cost_by_unit.as_ref(), priced),
        ),
        (
            "guardrail_cost_in_spend",
            optional(record.guardrail_cost_in_spend.as_ref(), |value: &bool| {
                Json::Bool(*value)
            }),
        ),
    ]
}

fn prompt_fields(record: &GuardrailInformation) -> [(&'static str, Option<Json>); 4] {
    let content = |value: &Json| value.clone();
    [
        (
            "guardrail_request",
            optional(record.guardrail_request.as_ref(), content),
        ),
        (
            "guardrail_response",
            optional(record.guardrail_response.as_ref(), content),
        ),
        (
            "match_details",
            optional(record.match_details.as_ref(), content),
        ),
        (
            "classification",
            optional(record.classification.as_ref(), content),
        ),
    ]
}

fn entries(value: &Json) -> Vec<&Object> {
    match value {
        Json::Object(fields) => vec![fields],
        Json::List(items) => items.iter().filter_map(as_object).collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => Vec::new(),
    }
}

fn redacted_record(record: &Object) -> Object {
    record
        .iter()
        .filter(|(field, _value)| classified(field))
        .map(|(field, value)| (field.clone(), classified_value(field, value)))
        .collect()
}

fn classified(field: &str) -> bool {
    if GUARDRAIL_PROMPT_FIELDS.contains(&field) {
        return true;
    }
    AUDIT_GUARDRAIL_FIELDS.contains(&field)
}

fn classified_value(field: &str, value: &Json) -> Json {
    if GUARDRAIL_PROMPT_FIELDS.contains(&field) {
        return Json::Text(REDACTED_BY_LITELM.to_owned());
    }
    value.clone()
}

// A field the payload carries as null stays null; one it leaves out stays out
fn optional<T, Convert>(field: Option<&Option<T>>, convert: Convert) -> Option<Json>
where
    Convert: Fn(&T) -> Json,
{
    match field {
        None => None,
        Some(None) => Some(Json::Null),
        Some(Some(value)) => Some(convert(value)),
    }
}

fn counted(counts: &IndexMap<String, i64>) -> Json {
    let mut object = Object::new();
    for (name, count) in counts {
        object.insert(name.clone(), json_number(Decimal::from(*count)));
    }
    Json::Object(object)
}

fn listed(items: &[String]) -> Json {
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        names.push(Json::Text(item.clone()));
    }
    Json::List(names)
}

fn priced(costs: &IndexMap<String, Option<DecimalValue>>) -> Json {
    let mut object = Object::new();
    for (unit, cost) in costs {
        let value = cost.map_or(Json::Null, |value| json_number(value.0));
        object.insert(unit.clone(), value);
    }
    Json::Object(object)
}

mod tests;
