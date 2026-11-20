use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Error as JsonError;
use serde_json::value::RawValue;
use serde_with::{FromInto, Map, serde_as};

// _decode_spend_log_row's record guard: nested values past this depth stop before the decoder recurses without bound.
const MAX_SPEND_LOG_DEPTH: usize = 64;

// SpendLogsPayload of litellm/proxy/_types.py: an open mapping, so the row keeps whatever columns the writer built.
pub type SpendLogRow = BTreeMap<String, SpendLogValue>;

// One column of a spend log row: the JSON values json.dumps writes on its own, with the Decimal that replaces Python's spend float.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SpendLogValue {
    Null,
    Bool(bool),
    Whole(i64),
    Decimal(#[serde_as(serialize_as = "FromInto<JsonAmount>")] Decimal),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl From<DateTime<Utc>> for SpendLogValue {
    fn from(moment: DateTime<Utc>) -> Self {
        // _spend_log_json_default: a datetime joins the row as its isoformat text, offset included and microseconds only when set.
        Self::Text(moment.to_rfc3339_opts(SecondsFormat::AutoSi, false))
    }
}

// A float's JSON form always keeps a decimal point, and a decimal literal without one is a float in Python's json.
#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct JsonAmount(#[serde(with = "rust_decimal::serde::arbitrary_precision")] Decimal);

impl From<Decimal> for JsonAmount {
    fn from(mut amount: Decimal) -> Self {
        if amount.scale() == 0 {
            amount.rescale(1);
        }
        Self(amount)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SpendLogError {
    #[error("a spend log row is not valid JSON")]
    Json(#[source] JsonError),
    #[error("a spend log value exceeds depth {MAX_SPEND_LOG_DEPTH}")]
    TooDeep,
    #[error("a spend log counter is outside the i64 range")]
    Counter,
    #[error("a spend log number is outside the decimal range")]
    Number(#[source] JsonError),
}

// _decode_spend_log_row: any JSON value that is not an object decodes to no row at all, while malformed JSON stays an error.
pub(super) fn decode_spend_log_row(bytes: &[u8]) -> Result<Option<SpendLogRow>, SpendLogError> {
    let raw: &RawValue = serde_json::from_slice(bytes).map_err(SpendLogError::Json)?;
    match decode_value(raw, MAX_SPEND_LOG_DEPTH)? {
        SpendLogValue::Object(row) => Ok(Some(row)),
        SpendLogValue::Null
        | SpendLogValue::Bool(_)
        | SpendLogValue::Whole(_)
        | SpendLogValue::Decimal(_)
        | SpendLogValue::Text(_)
        | SpendLogValue::Array(_) => Ok(None),
    }
}

fn decode_value(raw: &RawValue, remaining_depth: usize) -> Result<SpendLogValue, SpendLogError> {
    let text = raw.get();
    let next_depth = if matches!(text.as_bytes().first(), Some(b'[' | b'{')) {
        remaining_depth
            .checked_sub(1)
            .ok_or(SpendLogError::TooDeep)?
    } else {
        remaining_depth
    };
    match text.as_bytes().first() {
        Some(b'[') => {
            // Borrow children instead of allocating another copy of every nested subtree.
            let elements: Vec<&RawValue> =
                serde_json::from_str(text).map_err(SpendLogError::Json)?;
            return elements
                .into_iter()
                .map(|child| decode_value(child, next_depth))
                .collect::<Result<_, _>>()
                .map(SpendLogValue::Array);
        }
        Some(b'{') => {
            let fields: ObjectEntries<'_> =
                serde_json::from_str(text).map_err(SpendLogError::Json)?;
            // Validate overwritten entries too, so duplicate keys cannot hide excessive nesting.
            return fields
                .0
                .into_iter()
                .map(|(key, child)| decode_value(child, next_depth).map(|value| (key, value)))
                .collect::<Result<_, _>>()
                .map(SpendLogValue::Object);
        }
        Some(b'n') => Ok(SpendLogValue::Null),
        Some(b't' | b'f') => serde_json::from_str(text).map(SpendLogValue::Bool),
        Some(b'"') => serde_json::from_str(text).map(SpendLogValue::Text),
        _ => return decode_number(text),
    }
    .map_err(SpendLogError::Json)
}

fn decode_number(text: &str) -> Result<SpendLogValue, SpendLogError> {
    // json.loads reads a literal without a fraction or exponent as int and every other number as float.
    text.parse::<i64>().map_or_else(
        |_| {
            if !text.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E')) {
                return Err(SpendLogError::Counter);
            }
            serde_json::from_str::<ExactDecimal>(text)
                .map(|number| SpendLogValue::Decimal(number.0))
                .map_err(SpendLogError::Number)
        },
        |whole| Ok(SpendLogValue::Whole(whole)),
    )
}

#[derive(Deserialize)]
struct ExactDecimal(#[serde(with = "rust_decimal::serde::arbitrary_precision")] Decimal);

#[serde_as]
#[derive(Deserialize)]
struct ObjectEntries<'input>(
    #[serde_as(as = "Map<_, _>")]
    #[serde(borrow)]
    Vec<(String, &'input RawValue)>,
);
