use std::str::FromStr as _;

use betterlite_context::request_records::record::RequestRecord;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::converse_request::error::ConverseRequestError;
use crate::converse_request::records::{NUMBER_MARKER, TAG_LEAD};
use crate::model_routes::ParamValue;

// The inverse of record_value: a tagged literal or a Decimal becomes a JSON number, never text.
pub(super) fn value_of(record: &RequestRecord) -> Result<ParamValue, ConverseRequestError> {
    Ok(match record {
        RequestRecord::Null => ParamValue::Null,
        RequestRecord::Bool(flag) => ParamValue::Bool(*flag),
        RequestRecord::Signed(number) => ParamValue::Number(Number::from(*number)),
        RequestRecord::Unsigned(number) => ParamValue::Number(Number::from(*number)),
        RequestRecord::Decimal(number) => ParamValue::Number(decimal_number(number)?),
        RequestRecord::String(text) => tagged_value(text)?,
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

// The inverse of the tag: the marker after a stripped lead byte picks a number, and a doubled lead returns caller text.
fn tagged_value(text: &str) -> Result<ParamValue, ConverseRequestError> {
    let Some(tagged) = text.strip_prefix(TAG_LEAD) else {
        return Ok(ParamValue::Text(text.to_owned()));
    };
    let Some(literal) = tagged.strip_prefix(NUMBER_MARKER) else {
        return Ok(ParamValue::Text(tagged.to_owned()));
    };
    Ok(ParamValue::Number(Number::from_str(literal).map_err(
        |_| ConverseRequestError::FieldType {
            field: "params",
            expected: "a number this process can carry",
        },
    )?))
}
