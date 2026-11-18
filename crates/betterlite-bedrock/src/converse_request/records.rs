mod value;

use std::str::FromStr as _;

use betterlite_context::request_records::RequestRecordProjection;
use betterlite_context::request_records::record::RequestRecord;
use rust_decimal::Decimal;
use serde_json::Number;

use self::value::value_of;

use crate::converse_request::error::ConverseRequestError;
use crate::model_routes::{ParamMap, ParamValue};

pub(super) const MAX_DEPTH: i32 = 20;

// safe_deep_copy: the record layer owns the copy, and a JSON request never carries a live handle.
pub(super) fn safe_copy(params: &ParamMap) -> Result<ParamMap, ConverseRequestError> {
    let mut record = record_of(params)?;
    let copied = RequestRecordProjection::safe_copy(&mut record, false)?;
    params_of(&copied)
}

pub(super) fn record_of(params: &ParamMap) -> Result<RequestRecord, ConverseRequestError> {
    params
        .iter()
        .map(|(key, value)| Ok((key.clone(), record_value(value)?)))
        .collect::<Result<_, ConverseRequestError>>()
        .map(RequestRecord::Object)
}

fn record_value(value: &ParamValue) -> Result<RequestRecord, ConverseRequestError> {
    Ok(match value {
        ParamValue::Null => RequestRecord::Null,
        ParamValue::Bool(flag) => RequestRecord::Bool(*flag),
        ParamValue::Number(number) => number_record(number)?,
        ParamValue::Text(text) => RequestRecord::String(text.clone()),
        ParamValue::List(items) => RequestRecord::Array(
            items
                .iter()
                .map(record_value)
                .collect::<Result<_, ConverseRequestError>>()?,
        ),
        ParamValue::Mapping(fields) => RequestRecord::Object(
            fields
                .iter()
                .map(|(key, value)| Ok((key.clone(), record_value(value)?)))
                .collect::<Result<_, ConverseRequestError>>()?,
        ),
    })
}

// A float keeps its own scale in Decimal, so a parameter's spelling survives the record roundtrip.
fn number_record(number: &Number) -> Result<RequestRecord, ConverseRequestError> {
    if let Some(value) = number.as_i64() {
        return Ok(RequestRecord::Signed(value));
    }
    if let Some(value) = number.as_u64() {
        return Ok(RequestRecord::Unsigned(value));
    }
    let text = number.to_string();
    Decimal::from_str(&text)
        .or_else(|_| Decimal::from_scientific(&text))
        .map(RequestRecord::Decimal)
        .map_err(|_| ConverseRequestError::FieldType {
            field: "params",
            expected: "a number this process can carry",
        })
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
