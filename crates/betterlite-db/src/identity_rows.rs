use rust_decimal::prelude::ToPrimitive as _;
use serde::{Deserialize, Serialize, de::Error as _};
use std::collections::BTreeMap;
use std::str::{Utf8Error, from_utf8};
pub mod end_user;
pub mod key;
pub mod user;
use crate::budget_permission_rows::JsonData;
use end_user::{Expiry, LiteLLMEndUserTable};
use key::{DeletedVerificationToken, LiteLLMVerificationToken};
use user::{LiteLLMUserTable, ScimPlaceholder};
const MAX_ROW_BYTES: usize = 0x0010_0000;
const MAX_JSON_DEPTH: usize = 64;
pub type Payload = BTreeMap<String, JsonData>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "row")]
pub enum IdentityRow {
    #[serde(rename = "key")]
    Key(LiteLLMVerificationToken),
    #[serde(rename = "deleted_key")]
    DeletedKey(DeletedVerificationToken),
    #[serde(rename = "user")]
    User(LiteLLMUserTable),
    #[serde(rename = "placeholder")]
    Placeholder(ScimPlaceholder),
    #[serde(rename = "end_user")]
    EndUser(LiteLLMEndUserTable),
}
#[derive(Debug, thiserror::Error)]
pub enum IdentityRowsDecodeError {
    #[error("stored identity row is {len} bytes, over the {MAX_ROW_BYTES} byte limit")]
    TooLarge { len: usize },
    #[error("stored identity row is not valid UTF-8")]
    InvalidUtf8(#[source] Utf8Error),
    #[error("stored identity row exceeds JSON depth {MAX_JSON_DEPTH}")]
    TooDeep,
    #[error("stored identity row is invalid JSON")]
    Json(#[source] serde_json::Error),
}

pub fn decode_identity_rows(bytes: &[u8]) -> Result<IdentityRow, IdentityRowsDecodeError> {
    if bytes.len() > MAX_ROW_BYTES {
        return Err(IdentityRowsDecodeError::TooLarge { len: bytes.len() });
    }
    let text = from_utf8(bytes).map_err(IdentityRowsDecodeError::InvalidUtf8)?;
    check_depth(text)?;
    serde_json::from_str(text).map_err(IdentityRowsDecodeError::Json)
}
pub(crate) fn zero_decimal() -> rust_decimal::Decimal {
    let value = rust_decimal::Decimal::ZERO;
    value
        .checked_add(rust_decimal::Decimal::ZERO)
        .unwrap_or(value)
}
pub(crate) fn zero_i64() -> Option<i64> {
    let value = 0_i64;
    Option::from(value)
}
pub(crate) fn false_option() -> Option<bool> {
    let value = false;
    Option::from(value)
}
pub(crate) fn empty_option_vec<T>() -> Option<Vec<T>> {
    let value = Vec::new();
    Option::from(value)
}
pub(crate) fn empty_payload_option() -> Option<Payload> {
    let value = Payload::new();
    Option::from(value)
}
pub(crate) fn empty_vec<T>() -> Vec<T> {
    let mut value = Vec::new();
    value.shrink_to_fit();
    value
}
pub(crate) fn empty_vec_or_null<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    let value = Option::<Vec<T>>::deserialize(deserializer)?;
    Ok(value.unwrap_or_default())
}
pub(crate) fn null_decimal<'de, D>(deserializer: D) -> Result<rust_decimal::Decimal, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<rust_decimal::Decimal>::deserialize(deserializer)?;
    Ok(value.unwrap_or(rust_decimal::Decimal::ZERO))
}
pub(crate) fn integer_option<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IntegerValue {
        Integer(i64),
        Decimal(rust_decimal::Decimal),
        Text(String),
    }
    match Option::<IntegerValue>::deserialize(deserializer)? {
        None => Ok(None),
        Some(IntegerValue::Integer(value)) => Ok(Some(value)),
        Some(IntegerValue::Decimal(value)) => value
            .to_i64()
            .ok_or_else(|| D::Error::custom("expected an integer"))
            .map(Some),
        Some(IntegerValue::Text(value)) => value
            .parse::<i64>()
            .map(Some)
            .map_err(|error| D::Error::custom(error.to_string())),
    }
}
#[derive(Clone, Copy)]
enum ScanState {
    Structural,
    Quoted,
    Escaped,
}
fn check_depth(text: &str) -> Result<(), IdentityRowsDecodeError> {
    let mut state = ScanState::Structural;
    let mut depth = 0_u8;
    for byte in text.bytes() {
        match (state, byte) {
            (ScanState::Structural, b'{' | b'[') => {
                depth = depth.saturating_add(1);
                if usize::from(depth) > MAX_JSON_DEPTH {
                    return Err(IdentityRowsDecodeError::TooDeep);
                }
            }
            (ScanState::Structural, b'}' | b']') => depth = depth.saturating_sub(1),
            (ScanState::Structural, b'"') | (ScanState::Escaped, _) => state = ScanState::Quoted,
            (ScanState::Quoted, b'\\') => state = ScanState::Escaped,
            (ScanState::Quoted, b'"') => state = ScanState::Structural,
            _ => {}
        }
    }
    Ok(())
}
#[path = "identity_rows/tests.rs"]
mod tests;
