use std::error::Error;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use serde_json::{Map, Value};

use super::collect::{coerce, merged_hidden};
use super::error::DispatchError;
use super::input::{
    BridgeCall, Input, ModelResponseSeed, inject_stream, json_mode, resolve_stream_flag,
};
use super::port::{
    Answer, BridgeOutput, BridgeResponse, PostStreamDeps, ResponsesCall, ResponsesEventStream,
    ResponsesExecution,
};
use super::stream::{MockStream, StreamConfig, TranslatedStream, post_process};
use crate::api::api_boundaries::ApiLimits;
use crate::api::chat_response_stream::source::AsyncChunkStream;
use crate::api::response_chat_request::records::{ChatRequestInput, ResponsesChatRequest};
use crate::api::response_chat_request::request::build_request;
use crate::api::response_chat_result::records::ModelResponse;
use crate::api::response_chat_result::transform::{ResponseSource, transform_response};

#[derive(Clone, Copy)]
enum Mode {
    Sync,
    Async,
}

// litellm.completion() past the router, whose Responses answer is transformed back into a chat response
pub async fn completion<Execution, Client>(
    execution: &Execution,
    call: BridgeCall<'_, Client>,
) -> Result<BridgeOutput, DispatchError<Execution::Error>>
where
    Execution: ResponsesExecution<Client> + Sync,
    Client: ?Sized + Sync,
{
    run(execution, call, Mode::Sync).await
}

// litellm.acompletion(), which reaches aresponses() without the caller's client
pub async fn acompletion<Execution, Client>(
    execution: &Execution,
    call: BridgeCall<'_, Client>,
) -> Result<BridgeOutput, DispatchError<Execution::Error>>
where
    Execution: ResponsesExecution<Client> + Sync,
    Client: ?Sized + Sync,
{
    run(execution, call, Mode::Async).await
}

async fn run<Execution, Client>(
    execution: &Execution,
    call: BridgeCall<'_, Client>,
    mode: Mode,
) -> Result<BridgeOutput, DispatchError<Execution::Error>>
where
    Execution: ResponsesExecution<Client> + Sync,
    Client: ?Sized + Sync,
{
    let BridgeCall {
        kwargs,
        model_response,
        mut logging,
        client,
        limits,
        reasoning_auto_summary,
        post_stream,
    } = call;
    let mut input = super::input::input(kwargs)?;
    inject_stream(kwargs, &mut input.optional_params);
    let stream = resolve_stream_flag(&input.optional_params, &input.litellm_params);
    let client = match mode {
        Mode::Sync => client,
        Mode::Async => None,
    };
    let mut request = build_request(
        ChatRequestInput {
            model: &input.model,
            messages: &input.messages,
            optional_params: &mut input.optional_params,
            litellm_params: &input.litellm_params,
            headers: &input.headers,
            reasoning_auto_summary,
        },
        &mut logging,
        client,
    )?;
    route(&mut request, &input);
    let answer = execute(execution, &mut request, mode).await?;
    let delivery = Delivery {
        logging: request.logging,
        limits,
        post_stream,
        stream,
        json_mode: json_mode(kwargs),
    };
    deliver(&input, model_response, &delivery, answer).await
}

// The bridge sets both on request_data rather than as arguments, so the spread never repeats a keyword
fn route<Client: ?Sized>(request: &mut ResponsesChatRequest<'_, Client>, input: &Input) {
    request.litellm_params.insert(
        String::from("custom_llm_provider"),
        Value::String(input.custom_llm_provider.clone()),
    );
    request.params.model = format!("{}/{}", input.custom_llm_provider, input.model);
}

async fn execute<Execution, Client>(
    execution: &Execution,
    request: &mut ResponsesChatRequest<'_, Client>,
    mode: Mode,
) -> Result<Answer, DispatchError<Execution::Error>>
where
    Execution: ResponsesExecution<Client> + Sync,
    Client: ?Sized + Sync,
{
    let call = ResponsesCall {
        params: &request.params,
        litellm_params: &request.litellm_params,
        extra_headers: request.extra_headers.as_ref(),
        client: request.client,
        logging: &mut *request.logging,
    };
    let answer = match mode {
        Mode::Sync => execution.responses(call).await,
        Mode::Async => execution.aresponses(call).await,
    };
    answer.map_err(DispatchError::Provider)
}

struct Delivery<'delivery> {
    logging: &'delivery LoggingState,
    limits: ApiLimits,
    post_stream: Option<&'delivery PostStreamDeps<'delivery>>,
    stream: bool,
    json_mode: Option<bool>,
}

async fn deliver<Source>(
    input: &Input,
    seed: ModelResponseSeed,
    delivery: &Delivery<'_>,
    answer: Answer,
) -> Result<BridgeOutput, DispatchError<Source>>
where
    Source: Error + 'static,
{
    match answer {
        Answer::Response(response) => response_output(input, seed, delivery, response),
        Answer::Chat(chat) => chat_output(input, delivery, chat),
        Answer::Events(events) if delivery.stream => events_output(input, delivery, events),
        Answer::Events(events) => collected(input, seed, delivery, events).await,
        Answer::Cached(cached) if delivery.stream => post_process(
            cached,
            &input.model,
            &input.custom_llm_provider,
            delivery.post_stream,
        )
        .map(BridgeOutput::Stream)
        .map_err(DispatchError::PostStream),
        Answer::Cached(mut cached) => drain_cached(&mut *cached).await,
    }
}

// A collected stream keeps its response's own hidden keys over the iterator's, and B919 reads the merged map
fn response_output<Source>(
    input: &Input,
    seed: ModelResponseSeed,
    delivery: &Delivery<'_>,
    response: BridgeResponse,
) -> Result<BridgeOutput, DispatchError<Source>>
where
    Source: Error + 'static,
{
    let hidden = merged_hidden(&response.existing_hidden, &response.stream_hidden);
    let mut payload = coerce(response.payload)?;
    let result = ModelResponse {
        id: seed.id,
        created: seed.created,
        model: None,
        object: seed.object,
        system_fingerprint: None,
        choices: Vec::new(),
        usage: None,
        extra: Map::new(),
        hidden_params: Map::new(),
    };
    let transformed = transform_response(
        ResponseSource {
            model: &input.model,
            response: &mut payload,
            hidden_params: &hidden,
        },
        result,
        delivery.logging,
        &delivery.limits,
    )?;
    Ok(BridgeOutput::Response(Box::new(transformed)))
}

// A direct ModelResponse passes through, and streams only when the caller resolved a stream
fn chat_output<Source>(
    input: &Input,
    delivery: &Delivery<'_>,
    chat: Json,
) -> Result<BridgeOutput, DispatchError<Source>>
where
    Source: Error + 'static,
{
    if !delivery.stream {
        return Ok(BridgeOutput::Chat(chat));
    }
    let mock = MockStream::new(chat, &input.custom_llm_provider, &input.model)
        .map_err(|source| DispatchError::Mock { source })?;
    post_process(
        Box::new(mock),
        &input.model,
        &input.custom_llm_provider,
        delivery.post_stream,
    )
    .map(BridgeOutput::Stream)
    .map_err(DispatchError::PostStream)
}

fn events_output<Source>(
    input: &Input,
    delivery: &Delivery<'_>,
    events: Box<dyn ResponsesEventStream>,
) -> Result<BridgeOutput, DispatchError<Source>>
where
    Source: Error + 'static,
{
    let stream: Box<dyn AsyncChunkStream + Send> = Box::new(TranslatedStream::new(
        events,
        &StreamConfig {
            provider: &input.custom_llm_provider,
            model: &input.model,
            json_mode: delivery.json_mode,
            limits: delivery.limits,
        },
    ));
    post_process(
        stream,
        &input.model,
        &input.custom_llm_provider,
        delivery.post_stream,
    )
    .map(BridgeOutput::Stream)
    .map_err(DispatchError::PostStream)
}

async fn collected<Source>(
    input: &Input,
    seed: ModelResponseSeed,
    delivery: &Delivery<'_>,
    events: Box<dyn ResponsesEventStream>,
) -> Result<BridgeOutput, DispatchError<Source>>
where
    Source: Error + 'static,
{
    let response = super::collect::collect(events, &delivery.limits).await?;
    response_output(input, seed, delivery, response)
}

// A cached chat stream carries no completed Responses object, so a non-streaming caller gets the empty-stream error
async fn drain_cached<Source>(
    cached: &mut (dyn AsyncChunkStream + Send),
) -> Result<BridgeOutput, DispatchError<Source>>
where
    Source: Error + 'static,
{
    while (cached.next_chunk().await?).is_some() {}
    Err(DispatchError::MissingCompletedResponse)
}
