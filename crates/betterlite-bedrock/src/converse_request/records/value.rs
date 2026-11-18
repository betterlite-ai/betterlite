use std::str::FromStr as _;

use betterlite_context::request_records::record::RequestRecord;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::converse_request::error::ConverseRequestError;
use crate::model_routes::ParamValue;

// The inverse of record_value: a Decimal becomes its own spelling as a JSON number, never a string.
pub(super) fn value_of(record: &RequestRecord) -> Result<ParamValue, ConverseRequestError> {
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
