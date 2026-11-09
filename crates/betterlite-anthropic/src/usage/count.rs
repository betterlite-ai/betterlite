use std::error::Error as StdError;

use num_traits::cast;
use serde_json::Number;

use super::error::UsageError;
use super::wire::CountValue;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CountMode {
    // LiteLLM's top-level guard turns every non numeric shape into zero.
    Lenient,
    // A field the Anthropic TypedDicts declare as int: integers and booleans only.
    Declared,
    // sum(get(key, 0) or 0): falsey shapes become zero and other non integers fail.
    Iteration,
    // int(value or 0): numbers truncate and integer text reads.
    Coerced,
}

pub(super) fn count_value<CounterFailure>(
    value: Option<&CountValue>,
    field: &'static str,
    mode: CountMode,
) -> Result<i64, UsageError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    match value {
        None | Some(CountValue::Null) => Ok(0),
        Some(CountValue::Bool(flag)) => Ok(i64::from(*flag)),
        Some(CountValue::Number(number)) => match mode {
            CountMode::Declared | CountMode::Iteration => {
                if number.as_i64().is_none() && number.as_u64().is_none() {
                    return Err(UsageError::FieldType { field });
                }
                integer_literal(number).ok_or(UsageError::OutOfRange { field })
            }
            CountMode::Lenient | CountMode::Coerced => integer_literal(number)
                .or_else(|| number.as_f64().and_then(cast::<f64, i64>))
                .ok_or(UsageError::OutOfRange { field }),
        },
        Some(CountValue::Text(text)) => match mode {
            CountMode::Lenient => Ok(0),
            CountMode::Declared | CountMode::Iteration => Err(UsageError::FieldType { field }),
            CountMode::Coerced if text.is_empty() => Ok(0),
            CountMode::Coerced => text
                .trim()
                .parse::<i64>()
                .ok()
                .ok_or(UsageError::FieldType { field }),
        },
        Some(CountValue::List { empty } | CountValue::Object { empty }) => match mode {
            CountMode::Lenient => Ok(0),
            CountMode::Iteration | CountMode::Coerced if *empty => Ok(0),
            CountMode::Declared | CountMode::Iteration | CountMode::Coerced => {
                Err(UsageError::FieldType { field })
            }
        },
    }
}

pub(super) fn optional_int<CounterFailure>(
    value: Option<&CountValue>,
    field: &'static str,
) -> Result<Option<i64>, UsageError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    if value.is_none_or(|found| matches!(found, CountValue::Null)) {
        return Ok(None);
    }
    count_value(value, field, CountMode::Declared).map(Some)
}

pub(super) fn thinking_tokens(value: Option<&CountValue>) -> Option<i64> {
    match value {
        Some(CountValue::Number(number)) => integer_literal(number),
        Some(CountValue::Bool(flag)) => Some(i64::from(*flag)),
        None
        | Some(
            CountValue::Null
            | CountValue::Text(_)
            | CountValue::List { .. }
            | CountValue::Object { .. },
        ) => None,
    }
}

fn integer_literal(number: &Number) -> Option<i64> {
    if let Some(value) = number.as_i64() {
        return Some(value);
    }
    let value = number.as_u64()?;
    i64::try_from(value).ok()
}
