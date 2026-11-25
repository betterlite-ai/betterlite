use betterlite_anthropic::batches::BATCHES_ROUTE;
use betterlite_anthropic::batches::config::{AnthropicBatchesConfig, BatchesConfig as _};
use betterlite_anthropic::batches::timestamps::Clock;
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::{BatchObject, BatchStatus};
use chrono::{DateTime, Offset as _, Utc};
use indexmap::IndexMap;

use super::error::AnthropicBatchError;
use super::records::{CreationFailure, SuccessFields, creation_failed, success_logged};
use super::{
    ANTHROPIC_PREFIX, AnthropicBatchLogging, BatchCreationInput, BatchCreationLogged,
    SUCCESS_STATUS, UNKNOWN_MODEL, unified_object_id,
};
use crate::pass_through_logging::batch_attribution::{is_collection_route, optional_str};

pub(super) fn handle(
    logging: &AnthropicBatchLogging,
    input: &BatchCreationInput<'_>,
    call_log: &mut LoggingState,
) -> Result<BatchCreationLogged, AnthropicBatchError> {
    let body: Json = serde_json::from_slice(input.body)
        .map_err(|source| AnthropicBatchError::Parse { source })?;
    if !success_status(input.status_code, &body)? {
        let response_id = logging.ids.new_response_id();
        return Ok(creation_failed(
            input,
            response_id,
            CreationFailure::Status(input.status_code),
        ));
    }
    let batch_object = transform_batch(input.body, input.start_time)?;
    let batch_id = batch_object.id.clone();
    let model_name = request_model(input.request_body);
    let actual_model_id = logging.get_actual_model_id_from_router(&model_name);
    let prefixed_model_id = with_anthropic_prefix(actual_model_id, &model_name);
    let unified_id = unified_object_id(&prefixed_model_id, &batch_id);
    if is_collection_route(input.url_route, BATCHES_ROUTE) {
        logging.store_batch_managed_object(
            &unified_id,
            batch_object,
            &batch_id,
            input.request_metadata,
        );
    }
    let fields = SuccessFields {
        batch_id: &batch_id,
        model_name: &model_name,
        unified_object_id: &unified_id,
        response_id: logging.ids.new_response_id(),
    };
    Ok(success_logged(input, &fields, call_log))
}

fn success_status(status_code: u16, body: &Json) -> Result<bool, AnthropicBatchError> {
    if status_code != SUCCESS_STATUS {
        return Ok(false);
    }
    id_marker(body)
}

fn id_marker(body: &Json) -> Result<bool, AnthropicBatchError> {
    match body {
        Json::Object(fields) => Ok(fields.contains_key("id")),
        Json::List(items) => Ok(items
            .iter()
            .any(|item| matches!(item, Json::Text(text) if text == "id"))),
        Json::Text(text) => Ok(text.contains("id")),
        Json::Null => Err(AnthropicBatchError::UnsearchableBody { kind: "null" }),
        Json::Bool(_) => Err(AnthropicBatchError::UnsearchableBody { kind: "a boolean" }),
        Json::Number(_) => Err(AnthropicBatchError::UnsearchableBody { kind: "a number" }),
    }
}

fn transform_batch(
    body: &[u8],
    start_time: DateTime<Utc>,
) -> Result<BatchObject, AnthropicBatchError> {
    let config = AnthropicBatchesConfig {
        settings: AnthropicSettings::default(),
    };
    let clock = Clock {
        now: start_time.timestamp(),
        zone: Utc.fix(),
    };
    let mut batch = config.transform_retrieve_batch_response(body, &clock)?;
    batch.status = BatchStatus::Validating;
    Ok(batch)
}

fn request_model(request_body: Option<&Json>) -> String {
    let Some(Json::Object(body)) = request_body else {
        return UNKNOWN_MODEL.to_owned();
    };
    if let Some(model) = body.get("model").and_then(optional_str)
        && !model.is_empty()
        && model != UNKNOWN_MODEL
    {
        return model.to_owned();
    }
    nested_model(body).map_or_else(|| UNKNOWN_MODEL.to_owned(), str::to_owned)
}

fn nested_model(body: &IndexMap<String, Json>) -> Option<&str> {
    let Some(Json::List(requests)) = body.get("requests") else {
        return None;
    };
    let Some(Json::Object(first)) = requests.first() else {
        return None;
    };
    let Some(Json::Object(params)) = first.get("params") else {
        return None;
    };
    params
        .get("model")
        .and_then(optional_str)
        .filter(|model| !model.is_empty())
}

fn with_anthropic_prefix(actual_model_id: String, model_name: &str) -> String {
    if actual_model_id == model_name && !actual_model_id.starts_with(ANTHROPIC_PREFIX) {
        return format!("{ANTHROPIC_PREFIX}{model_name}");
    }
    actual_model_id
}
