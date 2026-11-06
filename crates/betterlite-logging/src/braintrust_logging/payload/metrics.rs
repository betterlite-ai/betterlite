use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use super::super::event::{CallbackPath, ResponseObject, SuccessEvent};
use super::PayloadError;
use super::fields::{object, optional_object};
use super::seconds;

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
    let start = Decimal::new(seconds::micros(event.start_time)?, 6);
    let end = Decimal::new(seconds::micros(event.end_time)?, 6);
    metrics.insert("start".to_owned(), seconds::text(start, "start")?);
    metrics.insert("end".to_owned(), seconds::text(end, "end")?);
    Ok(Some(Json::Object(metrics)))
}

// LiteLLM's sync callback times the whole call and its async callback times only to the first token
fn insert_first_token(
    metrics: &mut IndexMap<String, Json>,
    event: &SuccessEvent<'_>,
    path: CallbackPath,
) -> Result<(), PayloadError> {
    let first_token = match path {
        CallbackPath::Sync => Some(seconds::difference(
            event.end_time,
            event.start_time,
            "time_to_first_token",
        )?),
        CallbackPath::Async => match (event.completion_start_time, event.api_call_start_time) {
            (Some(completion), Some(call)) => Some(seconds::difference(
                completion,
                call,
                "time_to_first_token",
            )?),
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
            Ok(optional_object(fields.get("usage"), "response.usage")?.cloned())
        }
        Some(ResponseObject::TextCompletion(body) | ResponseObject::Embedding(body)) => {
            let fields = object(body, "response")?;
            let usage = optional_object(fields.get("usage"), "response.usage")?.cloned();
            Ok(Some(usage.unwrap_or_default()))
        }
    }
}

fn token(usage: &IndexMap<String, Json>, name: &'static str) -> Result<Json, PayloadError> {
    match usage.get(name) {
        None => Ok(Json::Number(Number::from(0_u8))),
        Some(Json::Number(number)) if number.is_i64() || number.is_u64() => {
            Ok(Json::Number(number.clone()))
        }
        Some(_) => Err(PayloadError::UsageToken { name }),
    }
}
