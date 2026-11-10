use betterlite_models::decisions::Json;

use crate::datadog_llm_obs::constants::{AUDIT_GUARDRAIL_FIELDS, GUARDRAIL_PROMPT_FIELDS};
use crate::datadog_llm_obs::values::{Object, as_object};
use crate::fields::payload::records::GuardrailInformation;
use crate::spend_sanitize::REDACTED_BY_LITELM;

use super::{RecordEncodingError, record_object};

// LiteLLM reads one record or a list of them, keeps the audit fields and marks the prompt-carrying ones
#[must_use]
pub fn redact_guardrails(value: Option<&Json>) -> Option<Vec<Object>> {
    let records = value.filter(|records| !matches!(records, Json::Null))?;
    let grouped = match records {
        Json::Object(fields) => vec![fields],
        Json::List(items) => items.iter().filter_map(as_object).collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => Vec::new(),
    };
    Some(grouped.into_iter().map(redacted_record).collect())
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

fn redacted_record(record: &Object) -> Object {
    record
        .iter()
        .filter(|(field, _value)| {
            GUARDRAIL_PROMPT_FIELDS.contains(&field.as_str())
                || AUDIT_GUARDRAIL_FIELDS.contains(&field.as_str())
        })
        .map(|(field, value)| (field.clone(), classified_value(field, value)))
        .collect()
}

fn classified_value(field: &str, value: &Json) -> Json {
    if GUARDRAIL_PROMPT_FIELDS.contains(&field) {
        return Json::Text(REDACTED_BY_LITELM.to_owned());
    }
    value.clone()
}

mod tests;
