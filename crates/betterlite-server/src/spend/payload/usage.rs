use betterlite_logging::logging_privacy::internal::{Origin, call_category, is_unbilled};
use betterlite_logging::spend_metadata::ocr::extract_usage_for_ocr_call;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

use super::{Context, SpendPayloadError, SpendResponse, as_object};

const TOKEN_TOTALS: [&str; 3] = ["completion_tokens", "prompt_tokens", "total_tokens"];
const PROMPT_DETAILS: &str = "prompt_tokens_details";
const CACHE_READ_FIELD: &str = "cache_read_input_tokens";
const CACHE_WRITE_FIELD: &str = "cache_creation_input_tokens";
const BACKGROUND_ORIGIN: &str = "background_response_cost_poll";

// The response's usage: OCR reads usage_info, an unbilled read reports none, and an empty answer lets the combined fill in.
pub(crate) fn project(context: &Context<'_>) -> Result<JsonObject, SpendPayloadError> {
    let response = match context.request.response {
        SpendResponse::Mapping(fields) => Some(fields),
        SpendResponse::Absent | SpendResponse::Other | SpendResponse::UnknownModelError => None,
    };
    let usage = if matches!(context.call_type, Some("ocr" | "aocr")) {
        let info = response.and_then(|fields| fields.get("usage_info"));
        extract_usage_for_ocr_call(info, None)
    } else if unbilled(context, response) {
        JsonObject::new()
    } else {
        response_usage(response)?
    };
    if !usage.is_empty() {
        return Ok(usage);
    }
    combined_usage(context.kwargs.get("combined_usage_object"))
}

fn unbilled(context: &Context<'_>, response: Option<&JsonObject>) -> bool {
    let background = matches!(
        response.and_then(|fields| fields.get("background")),
        Some(Json::Bool(true))
    );
    let origin = match context.metadata.get("internal_call_origin") {
        Some(Json::Text(origin)) if origin == BACKGROUND_ORIGIN => {
            Some(Origin::BackgroundResponseCostPoll)
        }
        Some(_) | None => None,
    };
    is_unbilled(call_category(context.call_type), origin, background)
}

fn response_usage(response: Option<&JsonObject>) -> Result<JsonObject, SpendPayloadError> {
    match response.and_then(|fields| fields.get("usage")) {
        None | Some(Json::Null) => Ok(JsonObject::new()),
        Some(Json::Object(fields)) => Ok(fields.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(SpendPayloadError::FieldType { field: "usage" })
        }
    }
}

// kwargs.combined_usage_object is a Usage model: only a mapping is one, and null means the stream recovered nothing.
fn combined_usage(value: Option<&Json>) -> Result<JsonObject, SpendPayloadError> {
    let Some(value) = value else {
        return Ok(JsonObject::new());
    };
    match value {
        Json::Null => Ok(JsonObject::new()),
        Json::Object(fields) => Ok(fields.clone()),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(SpendPayloadError::FieldType {
                field: "combined_usage_object",
            })
        }
    }
}

// Every usage field but the three token totals, with the cache counts filled from the prompt details.
pub(crate) fn additional_values(usage: &JsonObject, usage_object: Option<&Json>) -> JsonObject {
    let mut values: JsonObject = usage
        .iter()
        .filter(|(key, _)| !TOKEN_TOTALS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let Some(details) = details_source(&values, usage_object) else {
        return values;
    };
    if !values.contains_key(CACHE_READ_FIELD)
        && let Some(count) = positive_integer(details.get("cached_tokens"))
    {
        values.insert(
            CACHE_READ_FIELD.to_owned(),
            Json::Number(Number::from(count)),
        );
    }
    if !values.contains_key(CACHE_WRITE_FIELD)
        && let Some(count) = positive_integer(preferred_write(details))
    {
        values.insert(
            CACHE_WRITE_FIELD.to_owned(),
            Json::Number(Number::from(count)),
        );
    }
    values
}

// The response's prompt details when they are a mapping, else the clean usage_object's.
fn details_source<'source>(
    values: &'source JsonObject,
    usage_object: Option<&'source Json>,
) -> Option<&'source JsonObject> {
    if let Some(Json::Object(details)) = values.get(PROMPT_DETAILS) {
        return Some(details);
    }
    usage_object
        .and_then(as_object)
        .and_then(|fields| fields.get(PROMPT_DETAILS))
        .and_then(as_object)
}

// prompt_tokens_details.get("cache_write_tokens") or .get("cache_creation_tokens"): truthiness picks the first.
fn preferred_write(details: &JsonObject) -> Option<&Json> {
    details
        .get("cache_write_tokens")
        .filter(|value| carries_write(value))
        .or_else(|| details.get("cache_creation_tokens"))
}

// Only a positive integer counts: Python's isinstance(int) also admits bools, which the declared int fields exclude.
fn positive_integer(value: Option<&Json>) -> Option<i64> {
    match value {
        Some(Json::Number(number)) => number.as_i64().filter(|count| *count > 0),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn carries_write(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
