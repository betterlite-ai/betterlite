use serde::Deserialize;
use serde::de::{DeserializeOwned, Deserializer, Error as _};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("the storage response is not a valid row list")]
pub struct RowsError(#[source] pub serde_json::Error);

pub fn decode_rows<T>(bytes: &[u8]) -> Result<Vec<T>, RowsError>
where
    T: DeserializeOwned,
{
    let decoded = serde_json::from_slice(bytes);
    decoded.map_err(RowsError)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BoolOrZeroOne {
    Bool(bool),
    Int(i8),
}

// ClickHouse reports booleans as the native type or as a 0/1 flag, matching pydantic's lax int-to-bool coercion.
pub fn bool_from_bool_or_zero_one<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    match BoolOrZeroOne::deserialize(deserializer)? {
        BoolOrZeroOne::Bool(flag) => Ok(flag),
        BoolOrZeroOne::Int(0) => Ok(false),
        BoolOrZeroOne::Int(1) => Ok(true),
        BoolOrZeroOne::Int(other) => Err(D::Error::custom(format!(
            "expected a boolean or 0/1, got {other}"
        ))),
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum IntOrNumericString {
    Int(i64),
    Text(String),
}

// ClickHouse reports 64-bit integers as quoted strings, matching pydantic's lax int coercion.
pub fn i64_from_int_or_numeric_string<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    match IntOrNumericString::deserialize(deserializer)? {
        IntOrNumericString::Int(value) => Ok(value),
        IntOrNumericString::Text(text) => text.parse::<i64>().map_err(|source| {
            D::Error::custom(format!(
                "expected an integer string, got {text:?}: {source}"
            ))
        }),
    }
}

mod tests;
