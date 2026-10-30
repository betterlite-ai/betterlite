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

// ClickHouse reports a flag as the native type, as a 0/1 int, or as a quoted digit string.
#[derive(Deserialize)]
#[serde(untagged)]
enum Lax {
    Bool(bool),
    Int(i64),
    Text(String),
}

trait FromLax: Sized {
    fn from_lax(value: Lax) -> Option<Self>;
}

impl FromLax for bool {
    fn from_lax(value: Lax) -> Option<Self> {
        match value {
            Lax::Bool(flag) => Some(flag),
            Lax::Int(0) => Some(false),
            Lax::Int(1) => Some(true),
            Lax::Int(_) | Lax::Text(_) => None,
        }
    }
}

impl FromLax for i64 {
    fn from_lax(value: Lax) -> Option<Self> {
        match value {
            Lax::Int(parsed) => Some(parsed),
            Lax::Text(text) => text.parse().ok(),
            Lax::Bool(_) => None,
        }
    }
}

pub fn bool_from_bool_or_zero_one<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Lax::deserialize(deserializer)?;
    bool::from_lax(value).ok_or_else(|| D::Error::custom("expected a boolean or 0/1"))
}

pub fn i64_from_int_or_numeric_string<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Lax::deserialize(deserializer)?;
    i64::from_lax(value).ok_or_else(|| D::Error::custom("expected an integer or a numeric string"))
}

mod tests;
