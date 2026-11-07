use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::Number;

use super::PayloadError;
use super::object;
use super::{CallbackPath, ResponseObject, SuccessEvent};

#[derive(Serialize)]
pub(super) struct Metrics<'call> {
    prompt_tokens: Number,
    completion_tokens: Number,
    total_tokens: Number,
    total_cost: &'call Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_to_first_token: Option<Number>,
    start: Number,
    end: Number,
}

pub(super) fn metrics<'call>(
    event: &SuccessEvent<'call>,
    path: CallbackPath,
) -> Result<Option<Metrics<'call>>, PayloadError> {
    let Some(usage) = usage(event)? else {
        return Ok(None);
    };
    let first_token = match path {
        CallbackPath::Sync => Some(difference(
            event.end_time,
            event.start_time,
            "time_to_first_token",
        )?),
        CallbackPath::Async => match (event.completion_start_time, event.api_call_start_time) {
            (Some(completion), Some(call)) => {
                Some(difference(completion, call, "time_to_first_token")?)
            }
            _ => None,
        },
    };
    Ok(Some(Metrics {
        prompt_tokens: token(&usage, "prompt_tokens")?,
        completion_tokens: token(&usage, "completion_tokens")?,
        total_tokens: token(&usage, "total_tokens")?,
        total_cost: event.response_cost.unwrap_or(&Json::Null),
        time_to_first_token: first_token,
        start: text(Decimal::new(micros(event.start_time)?, 6), "start")?,
        end: text(Decimal::new(micros(event.end_time)?, 6), "end")?,
    }))
}

fn usage(event: &SuccessEvent<'_>) -> Result<Option<IndexMap<String, Json>>, PayloadError> {
    match &event.response {
        None | Some(ResponseObject::Image(_)) => Ok(None),
        Some(ResponseObject::Model(body)) => {
            let fields = object(body, "response")?;
            Ok(fields
                .get("usage")
                .filter(|value| !matches!(value, Json::Null))
                .map(|value| object(value, "response.usage"))
                .transpose()?
                .cloned())
        }
        Some(ResponseObject::TextCompletion(body) | ResponseObject::Embedding(body)) => {
            let fields = object(body, "response")?;
            let usage = fields
                .get("usage")
                .filter(|value| !matches!(value, Json::Null))
                .map(|value| object(value, "response.usage"))
                .transpose()?
                .cloned();
            Ok(Some(usage.unwrap_or_default()))
        }
    }
}

fn token(usage: &IndexMap<String, Json>, name: &'static str) -> Result<Number, PayloadError> {
    let Some(value) = usage.get(name) else {
        return Ok(Number::from(0_u8));
    };
    if let Json::Number(number) = value
        && (number.is_i64() || number.is_u64())
    {
        return Ok(number.clone());
    }
    Err(PayloadError::UsageToken { name })
}

pub(super) fn micros(moment: DateTime<Utc>) -> Result<i64, PayloadError> {
    moment
        .timestamp()
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(i64::from(moment.timestamp_subsec_micros())))
        .ok_or(PayloadError::WrongType(
            "timestamp",
            "a representable instant",
        ))
}

pub(super) fn difference(
    later: DateTime<Utc>,
    earlier: DateTime<Utc>,
    field: &'static str,
) -> Result<Number, PayloadError> {
    let elapsed = Decimal::new(micros(later)?, 6)
        .checked_sub(Decimal::new(micros(earlier)?, 6))
        .ok_or(PayloadError::WrongType(field, "a representable duration"))?;
    text(elapsed, field)
}

pub(super) fn text(seconds: Decimal, field: &'static str) -> Result<Number, PayloadError> {
    let normalized = seconds.normalize().to_string();
    let value = if normalized.contains('.') {
        normalized
    } else {
        format!("{normalized}.0")
    };
    serde_json::from_str::<Number>(&value).map_err(|source| PayloadError::Number { field, source })
}
