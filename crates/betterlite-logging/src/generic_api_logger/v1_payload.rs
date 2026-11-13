use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
mod tests;
#[derive(Clone, Debug, Default)]
pub struct V1Kwargs {
    pub call_type: Option<String>,
    pub cache_hit: Option<bool>,
    pub model: Option<String>,
    pub user: Option<String>,
    pub model_parameters: Option<IndexMap<String, Json>>,
    pub messages: Option<Json>,
    pub metadata: Option<IndexMap<String, Json>>,
    pub cost: Option<Decimal>,
}
#[derive(Debug, thiserror::Error)]
pub enum V1PayloadError {
    #[error("the response is not a JSON object")]
    ResponseNotObject,
    #[error("the response carries no usage")]
    MissingUsage,
}

// docs/compatibility.md leaves Python's repr and str text out, so each value prints as its standard JSON text
#[must_use]
pub fn string_form(value: &Json) -> String {
    match value {
        Json::Null => "null".to_owned(),
        Json::Bool(flag) => flag.to_string(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) | Json::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}
fn object_form(object: Option<&IndexMap<String, Json>>) -> String {
    let Some(fields) = object else {
        return "{}".to_owned();
    };
    string_form(&Json::Object(fields.clone()))
}
fn messages_form(messages: Option<&Json>) -> String {
    let Some(value) = messages else {
        return "null".to_owned();
    };
    string_form(value)
}
// docs/compatibility.md gives timestamps as RFC 3339, so epoch seconds become a calendar time
fn timestamp_text(seconds: Decimal) -> String {
    let timestamp = seconds
        .checked_mul(Decimal::from(1_000_i64))
        .and_then(|value| value.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_millis);
    timestamp.map_or_else(String::new, |at| at.to_rfc3339())
}
fn id_form(fields: &IndexMap<String, Json>, generated_id: &str) -> String {
    match fields.get("id") {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null) | None => generated_id.to_owned(),
        Some(other) => string_form(other),
    }
}
pub fn v1_logging_payload(
    kwargs: &V1Kwargs,
    response: &Json,
    start_time: Decimal,
    end_time: Decimal,
    generated_id: &str,
) -> Result<IndexMap<String, String>, V1PayloadError> {
    let Json::Object(fields) = response else {
        return Err(V1PayloadError::ResponseNotObject);
    };
    let usage = fields.get("usage").ok_or(V1PayloadError::MissingUsage)?;
    let call_type = kwargs
        .call_type
        .clone()
        .unwrap_or_else(|| "litellm.completion".to_owned());
    let entries = [
        ("id", id_form(fields, generated_id)),
        ("call_type", call_type),
        (
            "cache_hit",
            string_form(&Json::Bool(kwargs.cache_hit.unwrap_or(false))),
        ),
        ("startTime", timestamp_text(start_time)),
        ("endTime", timestamp_text(end_time)),
        ("model", kwargs.model.clone().unwrap_or_default()),
        ("user", kwargs.user.clone().unwrap_or_default()),
        (
            "modelParameters",
            object_form(kwargs.model_parameters.as_ref()),
        ),
        ("messages", messages_form(kwargs.messages.as_ref())),
        ("response", string_form(response)),
        ("usage", string_form(usage)),
        ("metadata", object_form(kwargs.metadata.as_ref())),
        ("cost", kwargs.cost.unwrap_or_default().to_string()),
    ];
    Ok(entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect())
}
