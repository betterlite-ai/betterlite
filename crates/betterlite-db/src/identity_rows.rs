use rust_decimal::prelude::ToPrimitive as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize, de::Error as _, ser::Error as _};
use std::collections::BTreeMap;
use std::str::{Utf8Error, from_utf8};
pub mod end_user;
pub mod key;
pub mod user;
use crate::budget_permission_rows::JsonData;
use end_user::LiteLLMEndUserTable;
use key::{DeletedVerificationToken, LiteLLMVerificationToken};
use user::{LiteLLMUserTable, ScimPlaceholder};
const MAX_ROW_BYTES: usize = 0x0010_0000;
const MAX_JSON_DEPTH: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload(
    #[serde(
        deserialize_with = "serde_with::As::<serde_with::MapPreventDuplicates<serde_with::Same, serde_with::Same>>::deserialize"
    )]
    pub BTreeMap<String, JsonData>,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionalPayload(pub Option<Payload>);
impl Default for OptionalPayload {
    fn default() -> Self {
        Self(Some(Payload::default()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultFilledList<T>(pub Option<Vec<T>>);
impl<T> Default for DefaultFilledList<T> {
    fn default() -> Self {
        Self(Some(Vec::new()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZeroRotationCount(#[serde(deserialize_with = "integer_option")] pub Option<i64>);
impl Default for ZeroRotationCount {
    fn default() -> Self {
        Self(Some(0))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "LaxBool")]
pub struct PydanticBool(pub bool);
// Pydantic's lax mode reads these numeric and text spellings as booleans too.
#[derive(Deserialize)]
#[serde(untagged)]
enum LaxBool {
    Flag(bool),
    Integer(i64),
    Decimal(rust_decimal::Decimal),
    Text(String),
}
#[derive(Debug, thiserror::Error)]
#[error("the value is not a valid boolean")]
pub struct BoolError;
impl TryFrom<LaxBool> for PydanticBool {
    type Error = BoolError;
    fn try_from(input: LaxBool) -> Result<Self, BoolError> {
        match input {
            LaxBool::Flag(flag) => Ok(Self(flag)),
            LaxBool::Integer(0) => Ok(Self(false)),
            LaxBool::Integer(1) => Ok(Self(true)),
            LaxBool::Decimal(value) if value.fract().is_zero() => match value.to_i64() {
                Some(0) => Ok(Self(false)),
                Some(1) => Ok(Self(true)),
                _ => Err(BoolError),
            },
            LaxBool::Integer(_) | LaxBool::Decimal(_) => Err(BoolError),
            LaxBool::Text(text) => match text.to_ascii_lowercase().as_str() {
                "0" | "f" | "n" | "no" | "off" | "false" => Ok(Self(false)),
                "1" | "t" | "y" | "on" | "yes" | "true" => Ok(Self(true)),
                _ => Err(BoolError),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoRotateFlag(pub Option<PydanticBool>);
impl Default for AutoRotateFlag {
    fn default() -> Self {
        Self(Some(PydanticBool(false)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[derive(Deserialize)]
    struct ArbitraryDecimal(
        #[serde(with = "rust_decimal::serde::arbitrary_precision")] rust_decimal::Decimal,
    );
    let value = Option::<ArbitraryDecimal>::deserialize(deserializer)?;
    Ok(value.map_or(rust_decimal::Decimal::ZERO, |wrapper| wrapper.0))
}
// LiteLLM stores these amounts as a Python float, so this keeps the JSON number identical to Pydantic's output.
pub(crate) fn serialize_as_float<S>(
    value: &rust_decimal::Decimal,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let Some(as_float) = value.to_f64() else {
        return Err(S::Error::custom("decimal value does not fit in a float"));
    };
    serializer.serialize_f64(as_float)
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
        Some(IntegerValue::Decimal(value)) => {
            if !value.fract().is_zero() {
                return Err(D::Error::custom("expected an integer"));
            }
            value
                .to_i64()
                .ok_or_else(|| D::Error::custom("expected an integer"))
                .map(Some)
        }
        Some(IntegerValue::Text(value)) => value
            .parse::<i64>()
            .map(Some)
            .map_err(|error| D::Error::custom(error.to_string())),
    }
}
// Serde's library adapter creates this private, non-Debug value only during explicit serialization.
#[derive(Serialize)]
#[serde(transparent)]
pub struct ExposedSecret(String);
impl From<&SecretString> for ExposedSecret {
    fn from(secret: &SecretString) -> Self {
        Self(secret.expose_secret().to_owned())
    }
}
#[derive(Clone, Copy)]
enum ScanState {
    Structural,
    Quoted,
    Escaped,
}
fn advance_depth_scan(
    state: ScanState,
    byte: u8,
    depth: u8,
) -> Result<(ScanState, u8), IdentityRowsDecodeError> {
    match (state, byte) {
        (ScanState::Structural, b'{' | b'[') => {
            let next_depth = depth.saturating_add(1);
            if usize::from(next_depth) > MAX_JSON_DEPTH {
                return Err(IdentityRowsDecodeError::TooDeep);
            }
            Ok((ScanState::Structural, next_depth))
        }
        (ScanState::Structural, b'}' | b']') => {
            Ok((ScanState::Structural, depth.saturating_sub(1)))
        }
        (ScanState::Structural, b'"') | (ScanState::Escaped, _) => Ok((ScanState::Quoted, depth)),
        (ScanState::Quoted, b'\\') => Ok((ScanState::Escaped, depth)),
        (ScanState::Quoted, b'"') => Ok((ScanState::Structural, depth)),
        (ScanState::Structural | ScanState::Quoted, _) => Ok((state, depth)),
    }
}
fn check_depth(text: &str) -> Result<(), IdentityRowsDecodeError> {
    let mut state = ScanState::Structural;
    let mut depth = 0_u8;
    for byte in text.bytes() {
        (state, depth) = advance_depth_scan(state, byte, depth)?;
    }
    Ok(())
}
#[path = "identity_rows/tests.rs"]
mod tests;
