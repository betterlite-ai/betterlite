use std::error::Error;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use serde_json::Value;

use super::collect::{coerce, merged_hidden};
use super::error::DispatchError;
use super::input::{
    BridgeCall, Input, StreamStamps, inject_stream, json_mode, resolve_stream_flag,
};
use super::port::{
    Answer, BridgeOutput, BridgeResponse, BridgeStream, PostStreamDeps, ResponsesCall,
    ResponsesExecution,
};
use super::stream::{MockStream, StreamConfig, TranslatedStream, post_process};
use crate::api::api_boundaries::ApiLimits;
use crate::api::chat_response_stream::source::AsyncChunkStream;
use crate::api::response_chat_request::records::{ChatRequestInput, ResponsesChatRequest};
use crate::api::response_chat_request::request::build_request;
use crate::api::response_chat_result::records::ModelResponse;
use crate::api::response_chat_result::transform::{ResponseSource, transform_response};

// litellm.completion() past the router, whose Responses answer is transformed back into a chat response
pub async fn completion<Execution, Client>(
    execution: &Execution,
    call: BridgeCall<'_, Client>,
) -> Result<BridgeOutput, DispatchError<Execution::Error>>
where
    Execution: ResponsesExecution<Client> + Sync,
    Client: ?Sized + Sync,
{
    let asynchronous = matches!(call.kwargs.get("acompletion"), Some(Json::Bool(true)));
    run(execution, call, asynchronous).await
}

// The delivery stream flag is read after build_request, because the builder promotes extra_body keys into the optional params
async fn run<Execution, Client>(
    execution: &Execution,
    call: BridgeCall<'_, Client>,
    asynchronous: bool,
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
        stamps,
    } = call;
    let mut input = super::input::input(kwargs)?;
    inject_stream(kwargs, &mut input.optional_params);
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
        if asynchronous { None } else { client },
    )?;
    let stream = resolve_stream_flag(&input.optional_params, &input.litellm_params);
    route(&mut request, &input);
    let answer = execute(execution, &mut request, asynchronous)
        .await
        .map_err(DispatchError::Provider)?;
    let delivery = Delivery {
        logging: request.logging,
        limits,
        post_stream,
        stream,
        json_mode: json_mode(kwargs),
    };
    deliver(&input, model_response, &delivery, stamps, answer).await
}

// The bridge's sync and async entries take the same request, and only the execution differs
async fn execute<Execution, Client>(
    execution: &Execution,
    request: &mut ResponsesChatRequest<'_, Client>,
    asynchronous: bool,
) -> Result<Answer, Execution::Error>
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
    if asynchronous {
        execution.aresponses(call).await
    } else {
        execution.responses(call).await
    }
}

// The bridge sets both on request_data rather than as arguments, so the spread never repeats a keyword
fn route<Client>(request: &mut ResponsesChatRequest<'_, Client>, input: &Input)
where
    Client: ?Sized,
{
    request.litellm_params.insert(
        String::from("custom_llm_provider"),
        Value::String(input.custom_llm_provider.clone()),
    );
    request.params.model = format!("{}/{}", input.custom_llm_provider, input.model);
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
    model_response: ModelResponse,
    delivery: &Delivery<'_>,
    stamps: StreamStamps,
    answer: Answer,
) -> Result<BridgeOutput, DispatchError<Source>>
where
    Source: Error + 'static,
{
    let provider = input.custom_llm_provider.as_str();
    // _apply_post_stream_processing runs on whatever stream the bridge produced
    let stream = match answer {
        Answer::Response(response) => {
            return response_output(input, model_response, delivery, *response);
        }
        Answer::Chat(chat) if !delivery.stream => return Ok(BridgeOutput::Chat(chat)),
        Answer::Chat(chat) => BridgeStream::Typed(Box::new(MockStream::new(
            chat,
            provider,
            &input.model,
            stamps,
        ))),
        Answer::Events(events) if delivery.stream => {
            BridgeStream::Typed(Box::new(TranslatedStream::new(
                events,
                &StreamConfig {
                    provider,
                    model: &input.model,
                    json_mode: delivery.json_mode,
                    limits: delivery.limits,
                    stamps,
                },
            )))
        }
        Answer::Events(events) => {
            let response = super::collect::collect(events, &delivery.limits).await?;
            return response_output(input, model_response, delivery, response);
        }
        Answer::Cached(cached) if delivery.stream => BridgeStream::Wire(cached),
        Answer::Cached(mut cached) => return drain_cached(&mut *cached).await,
    };
    post_process(stream, &input.model, provider, delivery.post_stream)
        .map(BridgeOutput::Stream)
        .map_err(DispatchError::PostStream)
}

// A collected stream keeps its response's own hidden keys over the iterator's, and B919 reads the merged map
fn response_output<Source>(
    input: &Input,
    result: ModelResponse,
    delivery: &Delivery<'_>,
    response: BridgeResponse,
) -> Result<BridgeOutput, DispatchError<Source>>
where
    Source: Error + 'static,
{
    let hidden = merged_hidden(&response.existing_hidden, &response.stream_hidden);
    let mut payload = coerce(response.payload)?;
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

// A cached chat stream carries no completed Responses object, so a non-streaming caller gets the empty-stream error
async fn drain_cached<Source>(
    cached: &mut (dyn AsyncChunkStream + Send),
) -> Result<BridgeOutput, DispatchError<Source>>
where
    Source: Error + 'static,
{
    while cached
        .next_chunk()
        .await
        .map_err(|failure| DispatchError::Stream { failure })?
        .is_some()
    {}
    Err(DispatchError::MissingCompletedResponse)
}
