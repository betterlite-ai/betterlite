use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_core::routes::CallType;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use betterlite_openai::videos::reply::VideoObject;
use betterlite_prompts::arguments::json::Object;
use rust_decimal::Decimal;
use serde_json::{Map, Value};

use super::GeminiPassthroughError;
use super::cost::{CostRequest, CostResponse, PassthroughCompletionCost};

// ast-grep-ignore: no-long-names
pub(crate) fn create_gemini_response_logging_payload(
    cost: &dyn PassthroughCompletionCost,
    response: &mut Json,
    model: &str,
    mut kwargs: PassThroughLoggingKwargs,
    logging_obj: &StreamLoggingObject,
    custom_llm_provider: &str,
) -> Result<PassThroughLoggingKwargs, GeminiPassthroughError> {
    let response_cost = cost
        .completion_cost(&CostRequest {
            response: CostResponse::Completion(response),
            model,
            custom_llm_provider: "gemini",
            vertex_location: None,
            call_type: CallType::Completion,
        })
        .map_err(|source| GeminiPassthroughError::Cost {
            model: model.to_owned(),
            source,
        })?;
    kwargs
        .entries
        .insert("response_cost".to_owned(), decimal_json(response_cost));
    kwargs
        .entries
        .insert("model".to_owned(), Json::Text(model.to_owned()));
    kwargs.entries.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(custom_llm_provider.to_owned()),
    );
    let mut logged_model = model.to_owned();
    if let Json::Object(fields) = response {
        let call_id = logging_obj
            .read(&mut |state| state.litellm_call_id.clone())
            .unwrap_or_default();
        fields.insert("id".to_owned(), Json::Text(call_id));
        if let Some(Json::Text(name)) = fields.get("model")
            && !name.is_empty()
        {
            logged_model.clone_from(name);
        }
    }
    write_call_details(
        logging_obj,
        &logged_model,
        custom_llm_provider,
        response_cost,
    );
    Ok(kwargs)
}

// This factory is the only one that writes response_cost into the call details.
fn write_call_details(
    logging_obj: &StreamLoggingObject,
    model: &str,
    custom_llm_provider: &str,
    response_cost: Decimal,
) {
    logging_obj.edit(&mut |state| {
        state.model = model.to_owned();
        state
            .model_call_details
            .insert("model".to_owned(), Json::Text(model.to_owned()));
        state.model_call_details.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(custom_llm_provider.to_owned()),
        );
        state
            .model_call_details
            .insert("response_cost".to_owned(), decimal_json(response_cost));
    });
}

pub(crate) fn request_object(body: &Map<String, Value>) -> Result<Object, GeminiPassthroughError> {
    serde_json::from_value(Value::Object(body.clone()))
        .map_err(|source| GeminiPassthroughError::RequestBody { source })
}

// LiteLLM's model_dump excludes the video object's private hidden params, so the logged body is its fields.
pub(crate) fn video_body(video: &VideoObject) -> Result<Json, GeminiPassthroughError> {
    serde_json::to_value(video)
        .and_then(serde_json::from_value)
        .map_err(|source| GeminiPassthroughError::Encode { source })
}
