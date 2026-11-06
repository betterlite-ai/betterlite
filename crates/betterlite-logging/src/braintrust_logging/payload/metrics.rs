use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use super::super::event::{CallbackPath, ResponseObject, SuccessEvent};
use super::PayloadError;
use super::object;

pub(super) fn metrics(
    event: &SuccessEvent<'_>,
    path: CallbackPath,
) -> Result<Option<Json>, PayloadError> {
    let Some(usage) = usage(event)? else {
        return Ok(None);
    };
    let mut metrics = IndexMap::new();
    metrics.insert("prompt_tokens".to_owned(), token(&usage, "prompt_tokens")?);
    metrics.insert(
        "completion_tokens".to_owned(),
        token(&usage, "completion_tokens")?,
    );
    metrics.insert("total_tokens".to_owned(), token(&usage, "total_tokens")?);
    metrics.insert(
        "total_cost".to_owned(),
        event.response_cost.cloned().unwrap_or(Json::Null),
    );
    insert_first_token(&mut metrics, event, path)?;
    let start = Decimal::new(micros(event.start_time)?, 6);
    let end = Decimal::new(micros(event.end_time)?, 6);
    metrics.insert("start".to_owned(), text(start, "start")?);
    metrics.insert("end".to_owned(), text(end, "end")?);
    Ok(Some(Json::Object(metrics)))
}

// LiteLLM's sync callback times the whole call and its async callback times only to the first token
fn insert_first_token(
    metrics: &mut IndexMap<String, Json>,
    event: &SuccessEvent<'_>,
    path: CallbackPath,
) -> Result<(), PayloadError> {
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
    if let Some(value) = first_token {
        metrics.insert("time_to_first_token".to_owned(), value);
    }
    Ok(())
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

fn token(usage: &IndexMap<String, Json>, name: &'static str) -> Result<Json, PayloadError> {
    let Some(value) = usage.get(name) else {
        return Ok(Json::Number(Number::from(0_u8)));
    };
    if let Json::Number(number) = value
        && (number.is_i64() || number.is_u64())
    {
        return Ok(Json::Number(number.clone()));
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
) -> Result<Json, PayloadError> {
    let elapsed = Decimal::new(micros(later)?, 6)
        .checked_sub(Decimal::new(micros(earlier)?, 6))
        .ok_or(PayloadError::WrongType(field, "a representable duration"))?;
    text(elapsed, field)
}

// Python writes a timestamp or duration as the shortest decimal that round-trips, with a fraction digit.
pub(super) fn text(seconds: Decimal, field: &'static str) -> Result<Json, PayloadError> {
    let normalized = seconds.normalize().to_string();
    let value = if normalized.contains('.') {
        normalized
    } else {
        format!("{normalized}.0")
    };
    serde_json::from_str::<Number>(&value)
        .map(Json::Number)
        .map_err(|_source| PayloadError::WrongType(field, "a JSON number"))
}
