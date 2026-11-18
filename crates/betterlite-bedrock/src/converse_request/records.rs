mod value;

use betterlite_context::request_records::RequestRecordProjection;
use betterlite_context::request_records::record::RequestRecord;
use serde_json::Number;

use self::value::value_of;

use crate::converse_request::error::ConverseRequestError;
use crate::model_routes::{ParamMap, ParamValue};

pub(super) const MAX_DEPTH: i32 = 20;

// RequestRecord carries no float, so every non-integer number keeps its literal behind this tag and a caller string's leading tag byte is doubled.
pub(super) const TAG_LEAD: char = '\u{1}';
pub(super) const NUMBER_MARKER: &str = "num:";

// safe_deep_copy: the record layer owns the copy, and a JSON request never carries a live handle.
pub(super) fn safe_copy(params: &ParamMap) -> Result<ParamMap, ConverseRequestError> {
    let mut record = record_of(params);
    let copied = RequestRecordProjection::safe_copy(&mut record, false)?;
    params_of(&copied)
}

pub(super) fn record_of(params: &ParamMap) -> RequestRecord {
    RequestRecord::Object(
        params
            .iter()
            .map(|(key, value)| (key.clone(), record_value(value)))
            .collect(),
    )
}

fn record_value(value: &ParamValue) -> RequestRecord {
    match value {
        ParamValue::Null => RequestRecord::Null,
        ParamValue::Bool(flag) => RequestRecord::Bool(*flag),
        ParamValue::Number(number) => number_record(number),
        ParamValue::Text(text) => RequestRecord::String(escaped_text(text)),
        ParamValue::List(items) => RequestRecord::Array(items.iter().map(record_value).collect()),
        ParamValue::Mapping(fields) => RequestRecord::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), record_value(value)))
                .collect(),
        ),
    }
}

// Every other number keeps the exact literal the caller sent, which Decimal could change or reject.
fn number_record(number: &Number) -> RequestRecord {
    if let Some(value) = number.as_i64() {
        return RequestRecord::Signed(value);
    }
    if let Some(value) = number.as_u64() {
        return RequestRecord::Unsigned(value);
    }
    RequestRecord::String(format!("{TAG_LEAD}{NUMBER_MARKER}{number}"))
}

fn escaped_text(text: &str) -> String {
    if text.starts_with(TAG_LEAD) {
        return format!("{TAG_LEAD}{text}");
    }
    text.to_owned()
}

pub(super) fn params_of(record: &RequestRecord) -> Result<ParamMap, ConverseRequestError> {
    let RequestRecord::Object(fields) = record else {
        return Err(ConverseRequestError::FieldType {
            field: "params",
            expected: "an object",
        });
    };
    let mut params = ParamMap::new();
    for (key, value) in fields {
        params.insert(key.clone(), value_of(value)?);
    }
    Ok(params)
}
