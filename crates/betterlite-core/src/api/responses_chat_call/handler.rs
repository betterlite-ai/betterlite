use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::creation::system_instant;
use betterlite_streaming::stream_chunk_shaping::records::StreamOptions as WrapperStreamOptions;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::api::bfl::poll::PollSleeper;
use crate::api::chat_response_stream::driver::ChatResponseStream;
use crate::api::chat_response_stream_items::snapshot::StreamAssembly;
use crate::api::chat_response_stream_tools::state::StreamSetup;
use crate::api::responses_chat_output::assemble_response;
use crate::api::responses_chat_output::records::{ChatCompletionResponse, RequestFields};
use crate::api::responses_session_history::{ColdStorageReader, SessionHistory, SpendLogQuery};

use super::completions::{CompletionBody, CompletionClient, CompletionOutput};
use super::error::{AsyncCallError, CallError, ConversionError};
use super::prepare::{arguments, litellm_metadata, prepare};
use super::records::{
    CallOutcome, CompletionArguments, Delivery, Prepared, ResponseCall, ResponseChatCall,
    StreamIdentity,
};
use super::session::merged_fields;
use super::source::{ChunkSourceConfig, ResponsesChunkSource};

// LiteLLM's handler resolves no provider only when the model string carries one; a bare model is an OpenAI one
const DEFAULT_PROVIDER: &str = "openai";

// litellm.completion, literal: no B915 history lookup happens on this path
pub async fn response_handler<Completions, Context>(
    deps: &ResponseChatCall<'_, Completions>,
    call: ResponseCall<'_, Context>,
) -> Result<CallOutcome<Context>, CallError<Completions::Error>>
where
    Completions: CompletionClient,
    Context: StreamAssembly,
{
    let mut request = call;
    let prepared = prepare(deps, &mut request)?;
    let identity = StreamIdentity {
        model: request.model.to_owned(),
        custom_llm_provider: request.custom_llm_provider.map(str::to_owned),
    };
    let arguments = arguments(request.kwargs, &prepared.mapped_json)?;
    execute(deps, request, prepared, arguments, identity).await
}

// litellm.acompletion after the previous response's history is restored into the mapped messages
pub async fn async_response_handler<Completions, Query, Cold, Sleep, Context>(
    deps: &ResponseChatCall<'_, Completions>,
    history: &SessionHistory<'_, Query, Cold, Sleep>,
    call: ResponseCall<'_, Context>,
) -> Result<CallOutcome<Context>, AsyncCallError<Completions::Error, Cold::Error>>
where
    Completions: CompletionClient,
    Query: SpendLogQuery + Sync,
    Cold: ColdStorageReader + Sync,
    Sleep: PollSleeper + Sync,
    Context: StreamAssembly,
{
    let mut request = call;
    let mut prepared = prepare(deps, &mut request)?;
    // LiteLLM guards the lookup on a truthy previous_response_id, so an empty one never reaches the session hook
    if let Some(previous) = previous_response_id(&prepared.request)
        && !previous.is_empty()
    {
        prepared.mapped_json = merged_fields(history, previous, &prepared.mapped_json).await?;
    }
    let identity = mapped_identity(&prepared.mapped_json);
    let arguments = arguments(request.kwargs, &prepared.mapped_json)?;
    execute(deps, request, prepared, arguments, identity)
        .await
        .map_err(AsyncCallError::Call)
}

async fn execute<Completions, Context>(
    deps: &ResponseChatCall<'_, Completions>,
    call: ResponseCall<'_, Context>,
    prepared: Prepared,
    arguments: CompletionArguments,
    identity: StreamIdentity,
) -> Result<CallOutcome<Context>, CallError<Completions::Error>>
where
    Completions: CompletionClient,
    Context: StreamAssembly,
{
    let metadata = litellm_metadata(call.kwargs)?;
    let output = deps
        .completions
        .completion(&arguments)
        .await
        .map_err(CallError::Completion)?;
    finish(
        Delivery {
            input: prepared.input,
            request: prepared.request,
            stream_options: prepared.mapped.stream_options,
            identity,
            litellm_metadata: metadata,
            sources: call.sources,
            context: call.context,
            limits: deps.limits,
        },
        output,
    )
    .map_err(CallError::Conversion)
}

fn finish<Context>(
    delivery: Delivery<Context>,
    output: CompletionOutput,
) -> Result<CallOutcome<Context>, ConversionError>
where
    Context: StreamAssembly,
{
    match output {
        CompletionOutput::Response(chat) => response_outcome(&delivery, &chat),
        CompletionOutput::Stream(body) => stream_outcome(delivery, body),
        CompletionOutput::Unexpected { python_type } => {
            Err(ConversionError::UnexpectedResponse { python_type })
        }
    }
}

// The chat answer becomes the Responses object; request_input is only there because B914 keeps LiteLLM's signature
fn response_outcome<Context>(
    delivery: &Delivery<Context>,
    chat: &ChatCompletionResponse,
) -> Result<CallOutcome<Context>, ConversionError> {
    let encoded = serde_json::to_value(&delivery.request)
        .map_err(|source| ConversionError::Request { source })?;
    let request: RequestFields =
        serde_json::from_value(encoded).map_err(|source| ConversionError::Request { source })?;
    let assembled = assemble_response(&delivery.input, &request, chat, delivery.limits, || {
        (delivery.sources.ids)()
    })?;
    Ok(CallOutcome::Response(Box::new(assembled)))
}

// The provider's SSE bytes go through B004's wrapper into B924's iterator, which owns every Responses event
fn stream_outcome<Context>(
    delivery: Delivery<Context>,
    body: CompletionBody,
) -> Result<CallOutcome<Context>, ConversionError>
where
    Context: StreamAssembly,
{
    let source = ResponsesChunkSource::new(
        ChunkSourceConfig {
            provider: delivery
                .identity
                .custom_llm_provider
                .clone()
                .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned()),
            model: delivery.identity.model.clone(),
            stream_options: delivery.stream_options.map(|options| WrapperStreamOptions {
                include_usage: options.include_usage,
            }),
            now: system_instant,
            next_id: Uuid::new_v4,
        },
        body,
    );
    let stream = ChatResponseStream::new(
        StreamSetup {
            model: &delivery.identity.model,
            logging_context: delivery.context,
            request_input: &delivery.input,
            responses_api_request: &delivery.request,
            custom_llm_provider: delivery.identity.custom_llm_provider.as_deref(),
            litellm_metadata: delivery.litellm_metadata,
            limits: delivery.limits,
        },
        delivery.sources,
        source,
    )?;
    Ok(CallOutcome::Stream(Box::new(stream)))
}

fn previous_response_id(request: &Json) -> Option<&str> {
    let Json::Object(fields) = request else {
        return None;
    };
    match fields.get("previous_response_id") {
        Some(Json::Text(id)) => Some(id),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// LiteLLM's async iterator names itself after litellm_completion_request, defaulting an absent model to the empty string
fn mapped_identity(mapped: &Map<String, Value>) -> StreamIdentity {
    let model = match mapped.get("model") {
        Some(Value::String(text)) => text.clone(),
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_),
        )
        | None => String::new(),
    };
    let custom_llm_provider = match mapped.get("custom_llm_provider") {
        Some(Value::String(text)) => Some(text.clone()),
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_),
        )
        | None => None,
    };
    StreamIdentity {
        model,
        custom_llm_provider,
    }
}
