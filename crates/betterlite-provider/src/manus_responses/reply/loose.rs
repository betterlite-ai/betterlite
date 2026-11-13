use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::{
    ResponseApiUsage, ResponseOutputItem, ResponsesApiResponse,
};
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::responses_provider_contract::records::Params;

use super::super::ManusResponsesConfig;
use super::super::error::ManusResponsesError;
use super::{CREATED_AT_FIELD, ID_FIELD, OUTPUT_FIELD, REASONING_FIELD, TEXT_FIELD, USAGE_FIELD};

pub(super) fn zero_usage() -> Json {
    Json::Object(IndexMap::from([
        (String::from("input_tokens"), Json::Number(Number::from(0))),
        (String::from("output_tokens"), Json::Number(Number::from(0))),
        (String::from("total_tokens"), Json::Number(Number::from(0))),
    ]))
}

fn fallback_id(config: &ManusResponsesConfig, fields: &Params) -> String {
    match fields.get(ID_FIELD) {
        Some(Json::Text(id)) => id.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => config.unknown_id(),
    }
}

fn parseable<T: serde::de::DeserializeOwned>(value: &Json) -> bool {
    serde_json::to_value(value)
        .ok()
        .and_then(|raw| serde_json::from_value::<T>(raw).ok())
        .is_some()
}

fn object_or_empty(fields: &Params, name: &str) -> Json {
    match fields.get(name) {
        Some(Json::Object(entries)) => Json::Object(entries.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => Json::Object(IndexMap::new()),
    }
}

pub(super) fn construct_loose(
    config: &ManusResponsesConfig,
    fields: &Params,
) -> Result<ResponsesApiResponse, ManusResponsesError> {
    let mut record = IndexMap::new();
    record.insert(
        String::from(ID_FIELD),
        Json::Text(fallback_id(config, fields)),
    );
    record.insert(
        String::from(CREATED_AT_FIELD),
        fields
            .get(CREATED_AT_FIELD)
            .cloned()
            .unwrap_or_else(|| Json::Number(Number::from(0))),
    );
    let output = fields
        .get(OUTPUT_FIELD)
        .filter(|value| parseable::<Vec<ResponseOutputItem>>(value))
        .cloned()
        .unwrap_or_else(|| Json::List(Vec::new()));
    record.insert(String::from(OUTPUT_FIELD), output);
    record.insert(
        String::from(REASONING_FIELD),
        object_or_empty(fields, REASONING_FIELD),
    );
    record.insert(
        String::from(TEXT_FIELD),
        object_or_empty(fields, TEXT_FIELD),
    );
    let usage = fields
        .get(USAGE_FIELD)
        .filter(|value| parseable::<ResponseApiUsage>(value))
        .cloned()
        .unwrap_or_else(zero_usage);
    record.insert(String::from(USAGE_FIELD), usage);
    let value = serde_json::to_value(Json::Object(record))?;
    Ok(serde_json::from_value(value)?)
}
