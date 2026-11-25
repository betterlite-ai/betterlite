use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_logging::end_user::extraction::IdSettings;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;
use tracing::Level;

use super::super::response_costing::{
    ResponseCostError, compute_response_cost, resolve_costing_model,
};
use super::super::response_payload::end_user_from_request_body;
use super::AnthropicPassthroughLogger;

const ANTHROPIC: &str = "anthropic";
const COMPLETE_STREAMING_RESPONSE: &str = "complete_streaming_response";

pub(crate) struct AnthropicPayloadInput<'input> {
    pub(crate) response: &'input mut Json,
    pub(crate) model: &'input str,
    pub(crate) kwargs: PassThroughLoggingKwargs,
    pub(crate) call_log: &'input mut LoggingState,
    pub(crate) response_id: Option<&'input str>,
}

#[derive(Debug, Error)]
enum PayloadError {
    #[error(transparent)]
    Cost(#[from] ResponseCostError),
    #[error("kwargs litellm_params is not a JSON object")]
    LitellmParams,
    #[error("the kwargs could not be written as JSON: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("the logged response is not a JSON object")]
    ResponseNotObject,
}

// ast-grep-ignore: no-long-names - LiteLLM's _create_anthropic_response_logging_payload, shared by the whole and the streamed path
pub(crate) fn create_anthropic_response_logging_payload(
    logger: &AnthropicPassthroughLogger,
    mut input: AnthropicPayloadInput<'_>,
) -> PassThroughLoggingKwargs {
    store_streamed_response(input.call_log, input.response);
    if let Err(error) = payload_steps(logger, &mut input) {
        tracing::error!("Error creating Anthropic response logging payload: {error}");
    }
    input.kwargs
}

// Each step writes in place, so a later failure keeps what the earlier steps set.
fn payload_steps(
    logger: &AnthropicPassthroughLogger,
    input: &mut AnthropicPayloadInput<'_>,
) -> Result<(), PayloadError> {
    let model = resolve_costing_model(input.model, input.call_log);
    let response_cost =
        compute_response_cost(logger.cost.as_ref(), input.response, &model, input.call_log)?;
    let entries = &mut input.kwargs.entries;
    entries.insert("response_cost".to_owned(), decimal_json(response_cost));
    entries.insert("model".to_owned(), Json::Text(model.clone()));
    // the pass-through success path reads spend from the call log, not from kwargs
    input
        .call_log
        .model_call_details
        .insert("response_cost".to_owned(), decimal_json(response_cost));
    apply_end_user(logger, &mut input.kwargs)?;
    if tracing::enabled!(Level::DEBUG) {
        let pretty =
            serde_json::to_string_pretty(&input.kwargs.entries).map_err(PayloadError::Encode)?;
        tracing::debug!("kwargs= {pretty}");
    }
    stamp_identity(input, model)
}

// setdefault then update: proxy_server_request is replaced whole and the other litellm_params keys stay
fn apply_end_user(
    logger: &AnthropicPassthroughLogger,
    kwargs: &mut PassThroughLoggingKwargs,
) -> Result<(), PayloadError> {
    let Some(Json::Object(payload)) = kwargs.entries.get("passthrough_logging_payload") else {
        return Ok(());
    };
    let request_body = match payload.get("request_body") {
        Some(Json::Object(body)) => Some(body),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            None
        }
    };
    let settings = IdSettings {
        request_headers: &logger.request_headers,
        validate_end_user_id_in_db: false,
    };
    let Some(user) = end_user_from_request_body(request_body, &settings) else {
        return Ok(());
    };
    let mut body = IndexMap::new();
    body.insert("user".to_owned(), Json::Text(user));
    let mut request = IndexMap::new();
    request.insert("body".to_owned(), Json::Object(body));
    match kwargs.entries.get_mut("litellm_params") {
        Some(Json::Object(params)) => {
            params.insert("proxy_server_request".to_owned(), Json::Object(request));
        }
        None | Some(Json::Null) => {
            let mut params = IndexMap::new();
            params.insert("proxy_server_request".to_owned(), Json::Object(request));
            kwargs
                .entries
                .insert("litellm_params".to_owned(), Json::Object(params));
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(PayloadError::LitellmParams);
        }
    }
    Ok(())
}

// The response takes the upstream id or the call id and the costing model, and the call log records both.
fn stamp_identity(
    input: &mut AnthropicPayloadInput<'_>,
    model: String,
) -> Result<(), PayloadError> {
    let response_id = input
        .response_id
        .filter(|id| !id.is_empty())
        .map_or_else(|| input.call_log.litellm_call_id.clone(), str::to_owned);
    let Json::Object(fields) = &mut *input.response else {
        return Err(PayloadError::ResponseNotObject);
    };
    fields.insert("id".to_owned(), Json::Text(response_id));
    fields.insert("model".to_owned(), Json::Text(model.clone()));
    let details = &mut input.call_log.model_call_details;
    details.insert("model".to_owned(), Json::Text(model));
    let provider_set = match details.get("custom_llm_provider") {
        Some(Json::Text(provider)) => !provider.is_empty(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => true,
        None | Some(Json::Null) => false,
    };
    if !provider_set {
        details.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(ANTHROPIC.to_owned()),
        );
    }
    // LiteLLM mutates the one stored object; this copy is stored again so it carries the final id and model.
    store_streamed_response(input.call_log, input.response);
    Ok(())
}

// perform_redaction scrubs this entry only for a stream, so a whole response never lands here.
fn store_streamed_response(call_log: &mut LoggingState, response: &Json) {
    if call_log.model_call_details.get("stream") == Some(&Json::Bool(true)) {
        call_log
            .model_call_details
            .insert(COMPLETE_STREAMING_RESPONSE.to_owned(), response.clone());
    }
}
