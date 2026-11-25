use std::error::Error as StdError;

use betterlite_context::non_default_params::Params;
use betterlite_interactions::types::response::InteractionsApiResponse;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use jiff::tz::TimeZone;

use super::async_iterator::InteractionsAsyncIterator;
use super::error::InteractionsHandlerError;
use super::operation::{AsyncResponsesOperation, ResponsesOperation};
use super::state::IteratorSetup;
use super::streams::{AsyncResponsesEventStream, ResponsesAnswer, SyncResponsesEventStream};
use super::sync_iterator::InteractionsSyncIterator;
use crate::api::responses_transform::request::{ResponsesRequest, to_responses_request};
use crate::api::responses_transform::response::to_interactions_response;

// interactions_api_handler's arguments: the iterator parts, the explicit stream flag, the extra kwargs and the zone.
pub struct HandlerRequest<'call> {
    pub setup: IteratorSetup<'call>,
    pub stream: Option<bool>,
    pub extras: &'call Params,
    pub zone: &'call TimeZone,
}

// async_interactions_api_handler's arguments: the request the caller already built, the iterator parts and the zone; the provider comes from the request, as in LiteLLM's async branch.
pub struct AsyncHandlerRequest<'call> {
    pub responses_request: &'call ResponsesRequest,
    pub setup: IteratorSetup<'call>,
    pub zone: &'call TimeZone,
}

// What a handler hands back: a complete Interactions response, or the iterator over the synthesized events.
pub enum InteractionsAnswer<Iterator> {
    Complete(Box<InteractionsApiResponse>),
    Stream(Box<Iterator>),
}

// The iterator kind an entry point wraps a stream in, so the dispatch body is written once.
trait StreamWrap<Error>
where
    Error: StdError + Send + Sync + 'static,
{
    type Stream;

    fn wrap(stream: Self::Stream, setup: &IteratorSetup<'_>) -> Self;
}

impl<Error> StreamWrap<Error> for InteractionsSyncIterator<Error>
where
    Error: StdError + Send + Sync + 'static,
{
    type Stream = Box<dyn SyncResponsesEventStream<Error = Error>>;

    fn wrap(stream: Self::Stream, setup: &IteratorSetup<'_>) -> Self {
        Self::new(stream, setup)
    }
}

impl<Error> StreamWrap<Error> for InteractionsAsyncIterator<Error>
where
    Error: StdError + Send + Sync + 'static,
{
    type Stream = Box<dyn AsyncResponsesEventStream<Error = Error>>;

    fn wrap(stream: Self::Stream, setup: &IteratorSetup<'_>) -> Self {
        Self::new(stream, setup)
    }
}

// interactions_api_handler: transform the Interactions request, call responses(), then wrap a stream or transform the complete response; litellm.responses itself is a runtime lookup in vars(litellm), so the sync creation call arrives as a trait object.
pub fn interactions_api_handler<Context, Error>(
    operation: &dyn ResponsesOperation<Context = Context, Error = Error>,
    request: &HandlerRequest<'_>,
    context: Context,
) -> Result<InteractionsAnswer<InteractionsSyncIterator<Error>>, InteractionsHandlerError<Error>>
where
    Error: StdError + Send + Sync + 'static,
{
    let responses_request = build_responses_request(request)?;
    let answer = operation
        .responses(&responses_request, context)
        .map_err(InteractionsHandlerError::Call)?;
    dispatched(answer, &request.setup, request.setup.model, request.zone)
}

// async_interactions_api_handler: await aresponses() with the request already built, then wrap a stream or transform the complete response.
pub async fn async_interactions_api_handler<Operation>(
    operation: &Operation,
    request: &AsyncHandlerRequest<'_>,
    context: Operation::Context,
) -> Result<
    InteractionsAnswer<InteractionsAsyncIterator<Operation::Error>>,
    InteractionsHandlerError<Operation::Error>,
>
where
    Operation: AsyncResponsesOperation,
{
    let answer = operation
        .aresponses(request.responses_request, context)
        .await
        .map_err(InteractionsHandlerError::Call)?;
    let setup = request
        .setup
        .with_provider(request_provider(request.responses_request));
    dispatched(answer, &setup, setup.model, request.zone)
}

// The dispatch both entry points share once the operation returns: a complete interaction, or the stream in its iterator.
fn dispatched<Error, Stream, Iterator>(
    answer: ResponsesAnswer<Stream>,
    setup: &IteratorSetup<'_>,
    model: &str,
    zone: &TimeZone,
) -> Result<InteractionsAnswer<Iterator>, InteractionsHandlerError<Error>>
where
    Error: StdError + Send + Sync + 'static,
    Iterator: StreamWrap<Error, Stream = Stream>,
{
    match answer {
        ResponsesAnswer::Complete(response) => Ok(InteractionsAnswer::Complete(
            complete_interaction(&response, model, zone)?,
        )),
        ResponsesAnswer::Stream(stream) => Ok(InteractionsAnswer::Stream(Box::new(
            Iterator::wrap(stream, setup),
        ))),
    }
}

// The request the sync path calls with: LiteLLM's kwargs update first, then B3497's Interactions-to-Responses transform.
fn build_responses_request<CallError>(
    request: &HandlerRequest<'_>,
) -> Result<ResponsesRequest, InteractionsHandlerError<CallError>>
where
    CallError: StdError + Send + Sync + 'static,
{
    let extras = responses_extras(request);
    to_responses_request(
        request.setup.model,
        request.setup.request_input,
        request.setup.optional_params,
        &extras,
    )
    .map_err(InteractionsHandlerError::Transform)
}

// The transform the complete path shares: B3497's response transform, named with the caller's model and read in the caller's zone.
fn complete_interaction<CallError>(
    response: &ResponsesApiResponse,
    model: &str,
    zone: &TimeZone,
) -> Result<Box<InteractionsApiResponse>, InteractionsHandlerError<CallError>>
where
    CallError: StdError + Send + Sync + 'static,
{
    let interaction = to_interactions_response(response, Some(model), zone)
        .map_err(InteractionsHandlerError::Transform)?;
    Ok(Box::new(interaction))
}

// The transform's kwargs: stream is always present, so the explicit argument wins over optional_params, and custom_llm_provider rides along when set.
fn responses_extras(request: &HandlerRequest<'_>) -> Params {
    let mut extras = request.extras.clone();
    extras.insert(
        String::from("stream"),
        request.stream.map_or(JsonValue::Null, JsonValue::Bool),
    );
    if let Some(provider) = request.setup.custom_llm_provider {
        extras.insert(
            String::from("custom_llm_provider"),
            JsonValue::Text(provider.to_owned()),
        );
    }
    extras
}

// LiteLLM's async path reads the provider back out of the built request instead of taking it as an argument.
fn request_provider(request: &ResponsesRequest) -> Option<&str> {
    match request.passthrough.get("custom_llm_provider") {
        Some(JsonValue::Text(provider)) => Some(provider.as_str()),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    }
}
