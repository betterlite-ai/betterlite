use betterlite_core::api::media_relay_logging::spend::MediaSpend;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::{
    PassThroughResponse, PassThroughResponseObject,
};
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::error::TinyfishError;
use super::records::{TinyfishRun, run_cost};
use super::{TINYFISH_MODEL_NAME, TINYFISH_PROVIDER};
use crate::media_relay_logging::write_spend;

// LiteLLM stamps the model and provider on the log object and the kwargs, and logs the parsed run as JSON text when there is one.
pub fn build_logging_payload(
    logging_obj: &StreamLoggingObject,
    run: Option<&TinyfishRun>,
    result: &str,
    kwargs: &PassThroughLoggingKwargs,
    cost_per_step: Decimal,
) -> Result<HandlerOutcome, TinyfishError> {
    let spend = MediaSpend {
        model: TINYFISH_MODEL_NAME.to_owned(),
        custom_llm_provider: TINYFISH_PROVIDER,
        response_cost: run_cost(run, cost_per_step),
    };
    write_spend(logging_obj, &spend);
    logging_obj.edit(&mut |state| {
        state.custom_llm_provider = Some(TINYFISH_PROVIDER.to_owned());
    });
    let response = match run {
        Some(submitted) => Json::Text(
            serde_json::to_string(submitted).map_err(|source| TinyfishError::Encode { source })?,
        ),
        None => Json::Text(result.to_owned()),
    };
    let mut outcome_kwargs = spend.kwargs(kwargs);
    if let Some(call_id) = logging_obj.read(&mut |state| state.litellm_call_id.clone()) {
        outcome_kwargs
            .entries
            .insert("litellm_call_id".to_owned(), Json::Text(call_id));
    }
    let params = litellm_params(kwargs, logging_obj);
    outcome_kwargs
        .entries
        .insert("litellm_params".to_owned(), params);
    Ok(HandlerOutcome {
        result: Some(PassThroughResponse::Standard(PassThroughResponseObject {
            response,
        })),
        kwargs: outcome_kwargs,
    })
}

// The poller paths pass no request kwargs, so SLO attribution reads the params stored with the request.
fn litellm_params(kwargs: &PassThroughLoggingKwargs, logging_obj: &StreamLoggingObject) -> Json {
    let stored = logging_obj
        .read(&mut |state| state.model_call_details.get("litellm_params").cloned())
        .flatten();
    for candidate in [kwargs.entries.get("litellm_params"), stored.as_ref()] {
        if let Some(Json::Object(entries)) = candidate
            && !entries.is_empty()
        {
            return Json::Object(entries.clone());
        }
    }
    Json::Object(IndexMap::new())
}
