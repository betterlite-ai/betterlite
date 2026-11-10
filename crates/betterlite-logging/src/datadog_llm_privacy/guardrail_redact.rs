use betterlite_models::decisions::Json;

use crate::datadog_llm_obs::constants::{AUDIT_GUARDRAIL_FIELDS, GUARDRAIL_PROMPT_FIELDS};
use crate::datadog_llm_obs::values::{Object, as_object};
use crate::fields::payload::records::GuardrailInformation;
use crate::spend_sanitize::REDACTED_BY_LITELM;

use super::{RecordEncodingError, record_object};

// LiteLLM keeps the audit fields and marks the prompt-carrying ones; a key in neither set is dropped
#[must_use]
pub fn redact_guardrails(value: Option<&Json>) -> Option<Vec<Object>> {
    let records = value?;
    let redacted: Vec<Object> = entries(records).into_iter().map(redacted_record).collect();
    Some(redacted)
}

pub fn redacted_records(
    records: &[GuardrailInformation],
) -> Result<Vec<Object>, RecordEncodingError> {
    let mut redacted = Vec::with_capacity(records.len());
    for record in records {
        let fields = record_object("guardrail", record)?;
        redacted.push(redacted_record(&fields));
    }
    Ok(redacted)
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

mod tests;
