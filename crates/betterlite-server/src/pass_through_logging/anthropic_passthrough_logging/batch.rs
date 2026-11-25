use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::super::anthropic_batch::{AnthropicBatchLogging, BatchCreationInput};
use super::{AnthropicPassthroughError, AnthropicPassthroughInput, logged_json};

// batch_creation_handler gets the caller's kwargs and returns them with its own keys set on top.
pub(super) fn batch_outcome(
    batch_logging: &AnthropicBatchLogging,
    input: AnthropicPassthroughInput<'_>,
    call_log: &mut LoggingState,
) -> Result<HandlerOutcome, AnthropicPassthroughError> {
    let request_metadata = request_metadata(&input.kwargs);
    let request_body = Json::Object(input.request_body.clone());
    let logged = batch_logging.batch_creation_handler(
        BatchCreationInput {
            status_code: input.status_code,
            body: input.response_text.as_bytes(),
            url_route: input.url_route,
            start_time: input.start_time,
            request_body: Some(&request_body),
            request_metadata: &request_metadata,
        },
        call_log,
    );
    let result = logged_json(&logged.result)?;
    let batch_job_state = logged_json(&logged.kwargs.batch_job_state)?;
    let mut kwargs = input.kwargs;
    let entries = &mut kwargs.entries;
    entries.insert(
        "response_cost".to_owned(),
        decimal_json(logged.kwargs.response_cost),
    );
    entries.insert("model".to_owned(), Json::Text(logged.kwargs.model));
    if let Some(batch_id) = logged.kwargs.batch_id {
        entries.insert("batch_id".to_owned(), Json::Text(batch_id));
    }
    if let Some(unified_object_id) = logged.kwargs.unified_object_id {
        entries.insert(
            "unified_object_id".to_owned(),
            Json::Text(unified_object_id),
        );
    }
    entries.insert("batch_job_state".to_owned(), batch_job_state);
    Ok(HandlerOutcome {
        result: Some(PassThroughResponse::ModelResponse(result)),
        kwargs,
    })
}

// kwargs["litellm_params"]["metadata"], where a missing or null level counts as empty
fn request_metadata(kwargs: &PassThroughLoggingKwargs) -> JsonObject {
    let Some(Json::Object(params)) = kwargs.entries.get("litellm_params") else {
        return JsonObject::new();
    };
    match params.get("metadata") {
        Some(Json::Object(metadata)) => metadata.clone(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            JsonObject::new()
        }
    }
}
