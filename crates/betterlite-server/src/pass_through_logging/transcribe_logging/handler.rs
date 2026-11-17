use betterlite_core::api::media_relay_logging::spend::MediaSpend;
use betterlite_logging::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::{
    PassThroughResponse, PassThroughResponseObject,
};
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::TranscribeLoggingError;
use super::{TRANSCRIBE_CUSTOM_LLM_PROVIDER, TranscribeHandlerInput, operation_from_headers};
use crate::media_relay_logging::write_spend;

pub(super) fn outcome(input: TranscribeHandlerInput<'_>) -> HandlerOutcome {
    let result = PassThroughResponse::Standard(PassThroughResponseObject {
        response: Json::Text(input.result.to_owned()),
    });
    let kwargs = match logged(&input) {
        Ok(kwargs) => kwargs,
        Err(error) => {
            tracing::error!("Error in Amazon Transcribe passthrough logging handler: {error}");
            input.kwargs
        }
    };
    HandlerOutcome {
        result: Some(result),
        kwargs,
    }
}

fn logged(
    input: &TranscribeHandlerInput<'_>,
) -> Result<PassThroughLoggingKwargs, TranscribeLoggingError> {
    let operation = operation_from_headers(input.request_headers);
    let spend = MediaSpend {
        model: format!("{TRANSCRIBE_CUSTOM_LLM_PROVIDER}/{operation}"),
        custom_llm_provider: TRANSCRIBE_CUSTOM_LLM_PROVIDER,
        response_cost: Some(input.response_cost),
    };
    write_spend(input.logging_obj, &spend);
    let provider = TRANSCRIBE_CUSTOM_LLM_PROVIDER.to_owned();
    input
        .logging_obj
        .edit(&mut |state| state.custom_llm_provider = Some(provider.clone()));
    let mut kwargs = spend.kwargs(&input.kwargs);
    kwargs
        .entries
        .insert(STANDARD_LOGGING_OBJECT.to_owned(), payload(input)?);
    Ok(kwargs)
}

fn payload(input: &TranscribeHandlerInput<'_>) -> Result<Json, TranscribeLoggingError> {
    let response = Json::Object(IndexMap::from([(
        "response".to_owned(),
        Json::Text(input.result.to_owned()),
    )]));
    let built = input
        .logging_obj
        .read(&mut |state| {
            input
                .payload_builder
                .build(state, &response, input.start_time, input.end_time)
        })
        .ok_or(TranscribeLoggingError::StateTaken)?
        .ok_or(TranscribeLoggingError::Payload)?;
    let text = serde_json::to_string(&built)
        .map_err(|source| TranscribeLoggingError::Encode { source })?;
    serde_json::from_str(&text).map_err(|source| TranscribeLoggingError::Encode { source })
}
