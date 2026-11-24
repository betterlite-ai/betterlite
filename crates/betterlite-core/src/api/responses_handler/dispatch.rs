use std::error::Error as StdError;

use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_context::non_default_params::Params;
use betterlite_interactions::types::response::InteractionsApiResponse;
use betterlite_models::controls::json_value::JsonValue;
use jiff::tz::TimeZone;

use super::async_iterator::InteractionsAsyncIterator;
use super::error::InteractionsHandlerError;
use super::operation::{AsyncResponsesOperation, ResponsesOperation};
use super::state::IteratorSetup;
use super::streams::{AsyncResponsesAnswer, SyncResponsesAnswer};
use super::sync_iterator::InteractionsSyncIterator;
use crate::api::responses_transform::request::{ResponsesRequest, to_responses_request};
use crate::api::responses_transform::response::to_interactions_response;

// interactions_api_handler's arguments: the caller's request, its extra kwargs, the explicit stream flag and what the transform needs from our side.
pub struct HandlerRequest<'call> {
    pub model: &'call str,
    pub input: Option<&'call JsonValue>,
    pub optional_params: &'call Params,
    pub custom_llm_provider: Option<&'call str>,
    pub stream: Option<bool>,
    pub extras: &'call Params,
    pub litellm_metadata: &'call Params,
    pub zone: &'call TimeZone,
    pub ids: &'call dyn IdSource,
}

// async_interactions_api_handler's arguments: the Responses request is the one the caller already built, so the transform never runs twice.
pub struct AsyncHandlerRequest<'call> {
    pub responses_request: &'call ResponsesRequest,
    pub model: &'call str,
    pub input: Option<&'call JsonValue>,
    pub optional_params: &'call Params,
    pub custom_llm_provider: Option<&'call str>,
    pub litellm_metadata: &'call Params,
    pub zone: &'call TimeZone,
    pub ids: &'call dyn IdSource,
}

// What the sync handler hands back: a complete Interactions response, or the iterator over the synthesized events.
pub enum InteractionsAnswer<Error>
where
    Error: StdError + Send + Sync + 'static,
{
    Complete(Box<InteractionsApiResponse>),
    Stream(Box<InteractionsSyncIterator<Error>>),
}

// What the async handler hands back: the same two shapes over the awaited iterator.
pub enum AsyncInteractionsAnswer<Error>
where
    Error: StdError + Send + Sync + 'static,
{
    Complete(Box<InteractionsApiResponse>),
    Stream(Box<InteractionsAsyncIterator<Error>>),
}

// interactions_api_handler: transform the Interactions request, call responses(), then wrap a stream or transform the complete response.
pub fn interactions_api_handler<Operation>(
    operation: &Operation,
    request: &HandlerRequest<'_>,
    context: Operation::Context,
) -> Result<InteractionsAnswer<Operation::Error>, InteractionsHandlerError<Operation::Error>>
where
    Operation: ResponsesOperation,
{
    let extras = responses_extras(request);
    let responses_request = to_responses_request(
        request.model,
        request.input,
        request.optional_params,
        &extras,
    )
    .map_err(InteractionsHandlerError::Transform)?;
    let answer = operation
        .responses(&responses_request, context)
        .map_err(InteractionsHandlerError::Call)?;
    match answer {
        SyncResponsesAnswer::Complete(response) => {
            let interaction =
                to_interactions_response(&response, Some(request.model), request.zone)
                    .map_err(InteractionsHandlerError::Transform)?;
            Ok(InteractionsAnswer::Complete(Box::new(interaction)))
        }
        SyncResponsesAnswer::Stream(stream) => {
            let setup = IteratorSetup {
                model: request.model,
                request_input: request.input,
                optional_params: request.optional_params,
                custom_llm_provider: request.custom_llm_provider,
                litellm_metadata: request.litellm_metadata,
                ids: request.ids,
            };
            Ok(InteractionsAnswer::Stream(Box::new(
                InteractionsSyncIterator::new(stream, &setup),
            )))
        }
    }
}

// async_interactions_api_handler: await aresponses() with the request already built, then wrap a stream or transform the complete response.
pub async fn async_interactions_api_handler<Operation>(
    operation: &Operation,
    request: &AsyncHandlerRequest<'_>,
    context: Operation::Context,
) -> Result<AsyncInteractionsAnswer<Operation::Error>, InteractionsHandlerError<Operation::Error>>
where
    Operation: AsyncResponsesOperation,
{
    let answer = operation
        .aresponses(request.responses_request, context)
        .await
        .map_err(InteractionsHandlerError::Call)?;
    match answer {
        AsyncResponsesAnswer::Complete(response) => {
            let interaction =
                to_interactions_response(&response, Some(request.model), request.zone)
                    .map_err(InteractionsHandlerError::Transform)?;
            Ok(AsyncInteractionsAnswer::Complete(Box::new(interaction)))
        }
        AsyncResponsesAnswer::Stream(stream) => {
            let setup = IteratorSetup {
                model: request.model,
                request_input: request.input,
                optional_params: request.optional_params,
                custom_llm_provider: request.custom_llm_provider,
                litellm_metadata: request.litellm_metadata,
                ids: request.ids,
            };
            Ok(AsyncInteractionsAnswer::Stream(Box::new(
                InteractionsAsyncIterator::new(stream, &setup),
            )))
        }
    }
}

// The transform's kwargs: stream is always present, so the explicit argument wins over optional_params, and custom_llm_provider rides along when set.
fn responses_extras(request: &HandlerRequest<'_>) -> Params {
    let mut extras = request.extras.clone();
    extras.insert(String::from("stream"), stream_value(request.stream));
    if let Some(provider) = request.custom_llm_provider {
        extras.insert(
            String::from("custom_llm_provider"),
            JsonValue::Text(provider.to_owned()),
        );
    }
    extras
}

fn stream_value(stream: Option<bool>) -> JsonValue {
    match stream {
        Some(flag) => JsonValue::Bool(flag),
        None => JsonValue::Null,
    }
}
