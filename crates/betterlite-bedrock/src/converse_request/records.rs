use std::collections::BTreeSet;
use std::str::FromStr as _;

use betterlite_context::request_records::RequestRecordProjection;
use betterlite_context::request_records::record::RequestRecord;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::converse_request::error::ConverseRequestError;
use crate::model_routes::{ParamMap, ParamValue};

const MAX_DEPTH: i32 = 20;

pub(super) fn filter_exceptions(params: &ParamMap) -> Result<ParamMap, ConverseRequestError> {
    let record = RequestRecordProjection::filter_parameters(record_of(params)?, MAX_DEPTH);
    params_of(&record)
}

pub(super) fn filter_internal(params: &ParamMap) -> Result<ParamMap, ConverseRequestError> {
    let record = RequestRecordProjection::filter_internal(record_of(params)?, &BTreeSet::new());
    params_of(&record)
}

// safe_deep_copy: the record layer owns the copy, and a JSON request never carries a live handle.
pub(super) fn safe_copy(params: &ParamMap) -> Result<ParamMap, ConverseRequestError> {
    let mut record = record_of(params)?;
    let copied = RequestRecordProjection::safe_copy(&mut record, false)?;
    params_of(&copied)
}

fn record_of(params: &ParamMap) -> Result<RequestRecord, ConverseRequestError> {
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

fn params_of(record: &RequestRecord) -> Result<ParamMap, ConverseRequestError> {
    let RequestRecord::Object(fields) = record else {
        return Err(ConverseRequestError::FieldType {
            field: "params",
            expected: "an object",
        });
    };
    fields
        .iter()
        .map(|(key, value)| Ok((key.clone(), value_of(value)?)))
        .collect()
}

fn value_of(record: &RequestRecord) -> Result<ParamValue, ConverseRequestError> {
    Ok(match record {
        RequestRecord::Null => ParamValue::Null,
        RequestRecord::Bool(flag) => ParamValue::Bool(*flag),
        RequestRecord::Signed(number) => ParamValue::Number(Number::from(*number)),
        RequestRecord::Unsigned(number) => ParamValue::Number(Number::from(*number)),
        RequestRecord::Decimal(number) => ParamValue::Number(decimal_number(number)?),
        RequestRecord::String(text) => ParamValue::Text(text.clone()),
        RequestRecord::Array(items) => ParamValue::List(
            items
                .iter()
                .map(value_of)
                .collect::<Result<_, ConverseRequestError>>()?,
        ),
        RequestRecord::Object(fields) => ParamValue::Mapping(
            fields
                .iter()
                .map(|(key, value)| Ok((key.clone(), value_of(value)?)))
                .collect::<Result<_, ConverseRequestError>>()?,
        ),
        // Deserializing a request never yields a handle; the sentinel only exists inside a live process.
        RequestRecord::Handle(_) => {
            return Err(ConverseRequestError::FieldType {
                field: "params",
                expected: "JSON",
            });
        }
    })
}

fn decimal_number(number: &Decimal) -> Result<Number, ConverseRequestError> {
    Number::from_str(&number.to_string()).map_err(|_| ConverseRequestError::FieldType {
        field: "params",
        expected: "a number this process can carry",
    })
}
