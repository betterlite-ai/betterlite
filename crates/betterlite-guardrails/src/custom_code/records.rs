use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::runtime::input_type_name;
use crate::generic_guardrail_contracts::GuardrailInputType;

const FLAG_STATUS: &str = "guardrail_flagged";
const RECORD_KEY: &str = "standard_logging_guardrail_information";

pub(super) struct FlagRecord {
    pub guardrail_name: String,
    pub reason: String,
    pub input_type: GuardrailInputType,
    pub metadata: BTreeMap<String, GuardrailValue>,
    pub started: Duration,
    pub ended: Duration,
}

// The six fields _process_result hands to the logging helper; the helper's shape is B3664's.
pub(super) fn append_flag_record(
    request_data: &mut BTreeMap<String, GuardrailValue>,
    record: &FlagRecord,
) {
    let bucket_key = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    push_flag_record(request_data, bucket_key, flag_entry(record));
}

fn flag_response(record: &FlagRecord) -> GuardrailValue {
    let mut response = BTreeMap::new();
    response.insert("action".to_owned(), GuardrailValue::Text("flag".to_owned()));
    response.insert(
        "reason".to_owned(),
        GuardrailValue::Text(record.reason.clone()),
    );
    response.insert(
        "input_type".to_owned(),
        GuardrailValue::Text(input_type_name(record.input_type).to_owned()),
    );
    response.insert(
        "metadata".to_owned(),
        GuardrailValue::Object(record.metadata.clone()),
    );
    GuardrailValue::Object(response)
}

fn flag_entry(record: &FlagRecord) -> GuardrailValue {
    let duration = record.ended.saturating_sub(record.started);
    GuardrailValue::Object(BTreeMap::from([
        (
            "guardrail_name".to_owned(),
            GuardrailValue::Text(record.guardrail_name.clone()),
        ),
        ("guardrail_response".to_owned(), flag_response(record)),
        (
            "guardrail_status".to_owned(),
            GuardrailValue::Text(FLAG_STATUS.to_owned()),
        ),
        (
            "start_time".to_owned(),
            GuardrailValue::Decimal(record.started.as_secs_f64()),
        ),
        (
            "end_time".to_owned(),
            GuardrailValue::Decimal(record.ended.as_secs_f64()),
        ),
        (
            "duration".to_owned(),
            GuardrailValue::Decimal(duration.as_secs_f64()),
        ),
    ]))
}

// Append under litellm_metadata when present, else metadata: a list grows, null starts one.
fn push_flag_record(
    request_data: &mut BTreeMap<String, GuardrailValue>,
    bucket_key: &str,
    entry: GuardrailValue,
) {
    let mut bucket = match request_data.remove(bucket_key) {
        Some(GuardrailValue::Object(bucket)) => bucket,
        _ => BTreeMap::new(),
    };
    let mut records = match bucket.remove(RECORD_KEY) {
        None | Some(GuardrailValue::Null) => Vec::new(),
        Some(GuardrailValue::Array(records)) => records,
        Some(other) => vec![other],
    };
    records.push(entry);
    bucket.insert(RECORD_KEY.to_owned(), GuardrailValue::Array(records));
    request_data.insert(bucket_key.to_owned(), GuardrailValue::Object(bucket));
}
