use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::callback_logger::error::LoggerError;
use crate::fields::payload::StandardLoggingPayload;
use crate::helicone::event::{LitellmParams, SuccessEvent};

// SuccessEvent is the kwargs the sync success handler hands HeliconeLogger.log_success, read out of the standard logging payload.
pub(super) fn success_event(payload: &StandardLoggingPayload) -> Result<SuccessEvent, LoggerError> {
    let start_time = utc_at(payload.start_time).ok_or_else(|| invalid_time("startTime"))?;
    let end_time = utc_at(payload.end_time).ok_or_else(|| invalid_time("endTime"))?;
    Ok(SuccessEvent {
        model: payload.model.clone(),
        messages: messages(payload),
        response: payload.response.clone().unwrap_or(Json::Null),
        start_time,
        end_time,
        litellm_params: LitellmParams {
            custom_llm_provider: payload.custom_llm_provider.clone().unwrap_or_default(),
            metadata: metadata_object(payload),
            // the standard logging payload carries no proxy_server_request, so the helicone_ header overrides cannot be read
            proxy_server_request: None,
        },
    })
}

fn messages(payload: &StandardLoggingPayload) -> Vec<Json> {
    match payload.messages.as_ref() {
        Some(Json::List(items)) => items.clone(),
        Some(single) => vec![single.clone()],
        None => Vec::new(),
    }
}

// The request's own metadata rides nested in the standard payload, and Helicone reads those keys flat.
fn metadata_object(payload: &StandardLoggingPayload) -> IndexMap<String, Json> {
    let Ok(Json::Object(mut fields)) =
        serde_json::to_value(&payload.metadata).and_then(serde_json::from_value::<Json>)
    else {
        tracing::warn!("the Helicone callback metadata could not be encoded");
        return IndexMap::new();
    };
    if let Some(Json::Object(requester)) = payload.metadata.requester_metadata.as_ref() {
        for (name, value) in requester {
            fields.insert(name.clone(), value.clone());
        }
    }
    fields
}

fn utc_at(seconds: Decimal) -> Option<DateTime<Utc>> {
    seconds
        .checked_mul(Decimal::from(1_000_i64))
        .and_then(|milliseconds| milliseconds.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_millis)
}

fn invalid_time(field: &str) -> LoggerError {
    LoggerError::Delivery {
        callback: "helicone".to_owned(),
        message: format!("the standard logging payload {field} is not a valid timestamp"),
    }
}
