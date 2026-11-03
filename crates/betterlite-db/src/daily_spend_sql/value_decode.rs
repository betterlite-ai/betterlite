use super::values::{DailySpendValueError, SpendColumnValue};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_with::{Map, serde_as};

pub const MAX_DAILY_VALUE_BYTES: usize = 0x0010_0000;
pub const MAX_DAILY_VALUE_DEPTH: usize = 64;

#[derive(Deserialize)]
struct ExactDecimal(#[serde(with = "rust_decimal::serde::arbitrary_precision")] Decimal);

#[serde_as]
#[derive(Deserialize)]
struct ObjectEntries<'input>(
    #[serde_as(as = "Map<_, _>")]
    #[serde(borrow)]
    Vec<(String, &'input RawValue)>,
);

pub fn decode_daily_value(bytes: &[u8]) -> Result<SpendColumnValue, DailySpendValueError> {
    if bytes.len() > MAX_DAILY_VALUE_BYTES {
        return Err(DailySpendValueError::TooLarge);
    }
    let raw: &RawValue = serde_json::from_slice(bytes).map_err(DailySpendValueError::Json)?;
    decode_value(raw, MAX_DAILY_VALUE_DEPTH)
}

fn decode_value(
    raw: &RawValue,
    remaining_depth: usize,
) -> Result<SpendColumnValue, DailySpendValueError> {
    let text = raw.get();
    let next_depth = if matches!(text.as_bytes().first(), Some(b'[' | b'{')) {
        remaining_depth
            .checked_sub(1)
            .ok_or(DailySpendValueError::TooDeep)?
    } else {
        remaining_depth
    };
    match text.as_bytes().first() {
        Some(b'[') => {
            // Borrow children instead of allocating another copy of every nested subtree.
            let elements: Vec<&RawValue> =
                serde_json::from_str(text).map_err(DailySpendValueError::Json)?;
            return elements
                .into_iter()
                .map(|child| decode_value(child, next_depth))
                .collect::<Result<_, _>>()
                .map(SpendColumnValue::Array);
        }
        Some(b'{') => {
            let fields: ObjectEntries<'_> =
                serde_json::from_str(text).map_err(DailySpendValueError::Json)?;
            // Validate overwritten entries too, so duplicate keys cannot hide excessive nesting.
            return fields
                .0
                .into_iter()
                .map(|(key, child)| decode_value(child, next_depth).map(|value| (key, value)))
                .collect::<Result<_, _>>()
                .map(SpendColumnValue::Object);
        }
        Some(b'n') => Ok(SpendColumnValue::Null),
        Some(b't' | b'f') => serde_json::from_str(text).map(SpendColumnValue::Bool),
        Some(b'"') => serde_json::from_str(text).map(SpendColumnValue::Text),
        _ => decode_number(text),
    }
    .map_err(DailySpendValueError::Json)
}

fn decode_number(text: &str) -> Result<SpendColumnValue, serde_json::Error> {
    // Only numeric tokens reach here; literal objects cannot mimic serde_json's numeric map encoding.
    text.parse::<i64>().map_or_else(
        |_| {
            serde_json::from_str::<ExactDecimal>(text)
                .map(|number| SpendColumnValue::Decimal(number.0))
        },
        |whole| Ok(SpendColumnValue::Int(whole)),
    )
}
