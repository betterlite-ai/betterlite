use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::{
    PassThroughResponse, PassThroughResponseObject,
};
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceLookup;
use thiserror::Error;

use crate::api::media_relay_logging::spend::MediaSpend;
use crate::openai_classification::parse_route;
use cost::response_cost;
use route::classify_route;

mod cost;
pub mod duration;
pub mod route;
mod tests;

const PROVIDER: &str = "azure_speech";
const SHORT_AUDIO_PATH_PREFIX: &str = "/speech/";
const BATCH_PATH_PREFIX: &str = "/speechtotext/";
const FAST_TRANSCRIPTION_PATH: &str = "/speechtotext/transcriptions:transcribe";
const SHORT_AUDIO_MODEL: &str = "short-audio";
const FAST_TRANSCRIPTION_MODEL: &str = "fast-transcription";
const BATCH_MODEL: &str = "batch-transcription";
const PRICING_MODEL: &str = "azure/speech/azure-stt";
const PRICING_PROVIDER: &str = "azure";
const TICKS_PER_SECOND: u32 = 10_000_000;
const MILLISECONDS_PER_SECOND: u32 = 1_000;

// The uploaded bytes are absent when the request body was already read, the RuntimeError LiteLLM bills as no upload.
pub struct AzureSpeechPassthroughRequest<'request> {
    pub logging_obj: &'request StreamLoggingObject,
    pub url_route: &'request str,
    pub response_body: &'request [u8],
    pub request_body: Option<&'request [u8]>,
    pub result: &'request str,
    pub kwargs: &'request PassThroughLoggingKwargs,
    pub lookup: &'request dyn PriceLookup,
}

#[derive(Debug, Error)]
pub enum AzureSpeechLoggingError {
    #[error(
        "the Azure Speech body field {field} value {value} is outside the billed duration range"
    )]
    FieldValue { field: &'static str, value: String },
    #[error("the Azure Speech billed duration is outside the decimal range")]
    DurationRange,
    #[error("the Azure Speech response cost is outside the decimal range")]
    CostRange,
}

// The handler never fails the forwarded call: whatever it cannot bill is logged and left out of the spend row.
#[must_use]
pub fn azure_speech_handler(request: &AzureSpeechPassthroughRequest<'_>) -> HandlerOutcome {
    match priced_outcome(request) {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::error!(%error, "error in the Azure Speech passthrough logging handler");
            HandlerOutcome {
                result: Some(PassThroughResponse::Standard(PassThroughResponseObject {
                    response: Json::Text(request.result.to_owned()),
                })),
                kwargs: request.kwargs.clone(),
            }
        }
    }
}

fn priced_outcome(
    request: &AzureSpeechPassthroughRequest<'_>,
) -> Result<HandlerOutcome, AzureSpeechLoggingError> {
    let route = classify_route(&parse_route(request.url_route).path);
    let spend = MediaSpend {
        model: route.model_name(),
        custom_llm_provider: PROVIDER,
        response_cost: Some(response_cost(
            route,
            request.request_body,
            request.response_body,
            request.lookup,
        )?),
    };
    record(request.logging_obj, &spend);
    Ok(HandlerOutcome {
        result: Some(PassThroughResponse::Standard(PassThroughResponseObject {
            response: Json::Text(request.result.to_owned()),
        })),
        kwargs: spend.kwargs(request.kwargs),
    })
}

// LiteLLM writes all three keys onto the logging object, and the shared logger reads the cost off model_call_details.
fn record(logging_obj: &StreamLoggingObject, spend: &MediaSpend) {
    let mut details = spend.entries();
    logging_obj.edit(&mut |state| {
        state.model.clone_from(&spend.model);
        state.custom_llm_provider = Some(spend.custom_llm_provider.to_owned());
        state.model_call_details.append(&mut details);
    });
}
