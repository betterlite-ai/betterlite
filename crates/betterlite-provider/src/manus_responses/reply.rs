use betterlite_models::call_helpers::headers::{HeaderSource, process_response_headers};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_events::RawResponse;
use betterlite_openai::response_events::created_at::convert_created_at;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::responses_provider_contract::error::{BaseLlmException, ResponsesContractError};
use crate::api::responses_provider_contract::records::{
    Headers, Params, ResponsesLogging, ResponsesUpstream,
};

mod loose;

use super::ManusResponsesConfig;
use super::error::ManusResponsesError;
use super::json;
use loose::{construct_loose, zero_usage};

const CREATED_AT_FIELD: &str = "created_at";
const CAMEL_CREATED_AT_FIELD: &str = "createdAt";
const REASONING_FIELD: &str = "reasoning";
const TEXT_FIELD: &str = "text";
const OUTPUT_FIELD: &str = "output";
const USAGE_FIELD: &str = "usage";
const ID_FIELD: &str = "id";
const HEADERS_KEY: &str = "headers";
const ADDITIONAL_HEADERS_KEY: &str = "additional_headers";
const LOG_EVENT_TYPE_KEY: &str = "log_event_type";
const ORIGINAL_RESPONSE_KEY: &str = "original_response";
const ADDITIONAL_ARGS_KEY: &str = "additional_args";

fn reply_failure(upstream: &ResponsesUpstream, text: &str) -> ManusResponsesError {
    ManusResponsesError::from(ResponsesContractError::BaseLlm(BaseLlmException {
        status_code: upstream.status_code,
        message: text.to_owned(),
        headers: Headers::new(),
    }))
}

fn parse_fields(upstream: &ResponsesUpstream, text: &str) -> Result<Params, ManusResponsesError> {
    match serde_json::from_str::<Json>(text) {
        Ok(Json::Object(fields)) => Ok(fields.into_iter().collect()),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(reply_failure(upstream, text))
        }
        Err(_source) => Err(reply_failure(upstream, text)),
    }
}

fn record_post_call(logging: &mut ResponsesLogging, text: &str) {
    logging.model_call_details.extend([
        (String::from("input"), Json::Null),
        (String::from("api_key"), Json::Null),
        (
            String::from(ORIGINAL_RESPONSE_KEY),
            Json::Text(text.to_owned()),
        ),
        (
            String::from(ADDITIONAL_ARGS_KEY),
            Json::Object(IndexMap::from([(
                String::from("complete_input_dict"),
                Json::Object(IndexMap::new()),
            )])),
        ),
        (
            String::from(LOG_EVENT_TYPE_KEY),
            Json::Text(String::from("post_api_call")),
        ),
    ]);
}

fn convert_created_at_fields(
    config: &ManusResponsesConfig,
    fields: &mut Params,
    upstream: &ResponsesUpstream,
    text: &str,
) -> Result<(), ManusResponsesError> {
    if fields.contains_key(CAMEL_CREATED_AT_FIELD) && !fields.contains_key(CREATED_AT_FIELD) {
        let camel = fields
            .get(CAMEL_CREATED_AT_FIELD)
            .cloned()
            .unwrap_or(Json::Null);
        let Ok(converted) = convert_created_at(&camel, config.now()) else {
            return Err(reply_failure(upstream, text));
        };
        fields.insert(
            String::from(CREATED_AT_FIELD),
            Json::Number(Number::from(converted)),
        );
    }
    if let Some(value) = fields.get(CREATED_AT_FIELD).cloned() {
        let Ok(converted) = convert_created_at(&value, config.now()) else {
            return Err(reply_failure(upstream, text));
        };
        fields.insert(
            String::from(CREATED_AT_FIELD),
            Json::Number(Number::from(converted)),
        );
    }
    Ok(())
}

fn insert_defaults(config: &ManusResponsesConfig, fields: &mut Params) {
    for name in [REASONING_FIELD, TEXT_FIELD] {
        if matches!(fields.get(name), None | Some(Json::Null)) {
            fields.insert(String::from(name), Json::Object(IndexMap::new()));
        }
    }
    if matches!(fields.get(OUTPUT_FIELD), None | Some(Json::Null)) {
        fields.insert(String::from(OUTPUT_FIELD), Json::List(Vec::new()));
    }
    if matches!(fields.get(USAGE_FIELD), None | Some(Json::Null)) {
        fields.insert(String::from(USAGE_FIELD), zero_usage());
    }
    if matches!(fields.get(ID_FIELD), None | Some(Json::Null)) {
        fields.insert(String::from(ID_FIELD), Json::Text(config.unknown_id()));
    }
}

fn validate(
    config: &ManusResponsesConfig,
    fields: &Params,
) -> Result<ResponsesApiResponse, ManusResponsesError> {
    let value = serde_json::to_value(json::object_of(fields))?;
    match serde_json::from_value::<ResponsesApiResponse>(value) {
        Ok(response) => Ok(response),
        Err(error) => {
            tracing::debug!(error = %error, "Error constructing ResponsesAPIResponse");
            construct_loose(config, fields)
        }
    }
}

pub(super) fn headers_of(upstream: &ResponsesUpstream) -> IndexMap<String, String> {
    upstream
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn finish(
    config: &ManusResponsesConfig,
    upstream: &ResponsesUpstream,
    logging: &mut ResponsesLogging,
    fields: &Params,
) -> Result<ResponsesApiResponse, ManusResponsesError> {
    let response = validate(config, fields)?;
    let headers = headers_of(upstream);
    let additional_headers = process_response_headers(HeaderSource::Upstream(&headers));
    logging.model_call_details.insert(
        String::from(HEADERS_KEY),
        json::headers_object(&upstream.headers),
    );
    logging.model_call_details.insert(
        String::from(ADDITIONAL_HEADERS_KEY),
        Json::Object(additional_headers),
    );
    Ok(response)
}

pub(super) fn transform(
    config: &ManusResponsesConfig,
    upstream: &ResponsesUpstream,
    logging: &mut ResponsesLogging,
) -> Result<ResponsesApiResponse, ManusResponsesError> {
    let text = String::from_utf8_lossy(&upstream.body).into_owned();
    record_post_call(logging, &text);
    let mut fields = parse_fields(upstream, &text)?;
    convert_created_at_fields(config, &mut fields, upstream, &text)?;
    insert_defaults(config, &mut fields);
    finish(config, upstream, logging, &fields)
}

pub(super) fn raw<'reply>(
    upstream: &'reply ResponsesUpstream,
    headers: &'reply IndexMap<String, String>,
    text: &'reply str,
) -> RawResponse<'reply> {
    RawResponse {
        status: upstream.status_code,
        headers,
        text,
    }
}
