use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::HeadroomError;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailStatus, GuardrailTracingDetail,
};
use crate::guardrail_records::sanitize::sanitize_guardrail_response;
use crate::headroom_settings::HEADROOM_GUARDRAIL_PROVIDER;

pub(crate) const UNAVAILABLE_MESSAGE: &str =
    "headroom compression unavailable; request forwarded uncompressed";

const APPLIED_KEY: &str = "applied_guardrails";
const LITELLM_METADATA_KEY: &str = "litellm_metadata";
const METADATA_KEY: &str = "metadata";
const RECORDS_KEY: &str = "standard_logging_guardrail_information";

pub(crate) struct Record<'a> {
    pub(crate) guardrail_name: Option<&'a str>,
    pub(crate) response: GuardrailValue,
    pub(crate) status: GuardrailStatus,
    pub(crate) start_time: f64,
    pub(crate) end_time: f64,
}

pub(crate) fn now() -> f64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs_f64(),
        Err(_) => 0.0_f64,
    }
}

pub(crate) fn unavailable() -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([(
        "error".to_owned(),
        GuardrailValue::Text(UNAVAILABLE_MESSAGE.to_owned()),
    )]))
}

pub(crate) fn append(
    request_data: &mut BTreeMap<String, GuardrailValue>,
    record: Record<'_>,
) -> Result<(), HeadroomError> {
    let entry = invocation(record)?;
    let key = bucket_key(request_data);
    let mut bucket = take_bucket(request_data, key);
    push_record(&mut bucket, entry);
    request_data.insert(key.to_owned(), GuardrailValue::Object(bucket));
    Ok(())
}

pub(crate) fn append_applied(
    request_data: &mut BTreeMap<String, GuardrailValue>,
    guardrail_name: Option<&str>,
) {
    let Some(name) = guardrail_name else {
        return;
    };
    let key = bucket_key(request_data);
    let mut bucket = take_bucket(request_data, key);
    push_applied(&mut bucket, name);
    request_data.insert(key.to_owned(), GuardrailValue::Object(bucket));
}

fn invocation(record: Record<'_>) -> Result<GuardrailValue, HeadroomError> {
    let response = sanitize_guardrail_response(record.response)?;
    let entry = GuardrailInvocationRecord {
        guardrail_name: record.guardrail_name.map(str::to_owned),
        guardrail_provider: Some(HEADROOM_GUARDRAIL_PROVIDER.to_owned()),
        guardrail_mode: None,
        guardrail_response: response,
        guardrail_status: record.status,
        start_time: Some(record.start_time),
        end_time: Some(record.end_time),
        duration: Some(record.end_time - record.start_time),
        masked_entity_count: None,
        tracing_detail: GuardrailTracingDetail::default(),
    };
    let encoded = serde_json::to_value(&entry)?;
    let decoded = serde_json::from_value(encoded)?;
    Ok(decoded)
}

fn bucket_key(request_data: &BTreeMap<String, GuardrailValue>) -> &'static str {
    if request_data.contains_key(LITELLM_METADATA_KEY) {
        LITELLM_METADATA_KEY
    } else {
        METADATA_KEY
    }
}

fn take_bucket(
    request_data: &mut BTreeMap<String, GuardrailValue>,
    key: &str,
) -> BTreeMap<String, GuardrailValue> {
    match request_data.remove(key) {
        Some(GuardrailValue::Object(fields)) => fields,
        _ => BTreeMap::new(),
    }
}

fn push_record(bucket: &mut BTreeMap<String, GuardrailValue>, entry: GuardrailValue) {
    let mut records = match bucket.remove(RECORDS_KEY) {
        None | Some(GuardrailValue::Null) => Vec::new(),
        Some(GuardrailValue::Array(records)) => records,
        Some(other) => vec![other],
    };
    records.push(entry);
    bucket.insert(RECORDS_KEY.to_owned(), GuardrailValue::Array(records));
}

fn push_applied(bucket: &mut BTreeMap<String, GuardrailValue>, name: &str) {
    let mut applied = match bucket.remove(APPLIED_KEY) {
        Some(GuardrailValue::Array(names)) => names,
        _ => Vec::new(),
    };
    let known = applied
        .iter()
        .any(|existing| matches!(existing, GuardrailValue::Text(text) if text == name));
    if !known {
        applied.push(GuardrailValue::Text(name.to_owned()));
    }
    bucket.insert(APPLIED_KEY.to_owned(), GuardrailValue::Array(applied));
}
