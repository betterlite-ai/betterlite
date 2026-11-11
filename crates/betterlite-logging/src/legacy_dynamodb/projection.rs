use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::fields::payload::StandardLoggingPayload;
use crate::generic_api_logger::v1_payload::string_form;

mod tests;

const MILLIS_PER_SECOND: i64 = 1_000;

// LiteLLM's dynamodb payload dict, with each value as a string so DynamoDB stores an S attribute
pub fn item(
    payload: &StandardLoggingPayload,
) -> Result<IndexMap<String, String>, serde_json::Error> {
    let mut item = IndexMap::new();
    item.insert("id".to_owned(), payload.id.clone());
    item.insert("call_type".to_owned(), payload.call_type.clone());
    item.insert("startTime".to_owned(), timestamp_text(payload.start_time));
    item.insert("endTime".to_owned(), timestamp_text(payload.end_time));
    item.insert("model".to_owned(), payload.model.clone());
    item.insert(
        "user".to_owned(),
        payload.end_user.clone().unwrap_or_default(),
    );
    item.insert(
        "modelParameters".to_owned(),
        string_form(&Json::Object(payload.model_parameters.clone())),
    );
    item.insert(
        "messages".to_owned(),
        payload
            .messages
            .as_ref()
            .map_or_else(|| string_form(&Json::Null), string_form),
    );
    item.insert(
        "response".to_owned(),
        payload
            .response
            .as_ref()
            .map_or_else(|| string_form(&Json::Null), string_form),
    );
    item.insert("usage".to_owned(), usage_text(payload));
    item.insert(
        "metadata".to_owned(),
        serde_json::to_string(&payload.metadata)?,
    );
    Ok(item)
}

fn usage_text(payload: &StandardLoggingPayload) -> String {
    if let Some(Json::Object(fields)) = payload.response.as_ref()
        && let Some(usage) = fields.get("usage")
    {
        return string_form(usage);
    }
    payload
        .metadata
        .usage_object
        .as_ref()
        .map_or_else(|| string_form(&Json::Null), string_form)
}

fn timestamp_text(seconds: Decimal) -> String {
    seconds
        .checked_mul(Decimal::from(MILLIS_PER_SECOND))
        .and_then(|value| value.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map_or_else(|| seconds.to_string(), |at| at.to_rfc3339())
}
