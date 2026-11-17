use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::text_completion_transport::sdk::ClientConstructor;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use indexmap::IndexMap;
use uuid::Uuid;

use crate::operation::{ChatCompletion, OperationError, OperationRequest, OperationResult};
use crate::provider::Adapter as _;

use super::call::{ChatCall, Composition, EmbeddingCall, OpenAiOperation, TextCall};
use super::compose::{ChatPlan, chat_plan, embedding_plan};
use super::error::OpenAiProviderError;
use super::logging::{LoggedCall, OpenAiLogger, logged_json};
use super::text::{self, TextOutcome};
use super::transport::{Deps, ProviderAnswer, TextClientSource};
use super::{OpenAi, OpenAiRequest, OpenAiResult};

// The closed provider entry: only the OpenAI request variant reaches these branches, and the operation names the leaf.
pub async fn dispatch<Answer, Source, Logger>(
    request: OperationRequest,
    operation: OpenAiOperation,
    composition: &Composition,
    deps: Deps<'_, Answer, Source>,
    logger: &mut Logger,
) -> Result<OperationResult, OpenAiProviderError>
where
    Answer: ProviderAnswer,
    Source: TextClientSource,
    <Source::Constructor as ClientConstructor>::Client: Clone,
    Logger: OpenAiLogger + Send,
{
    let OperationRequest::OpenAi(OpenAiRequest) = request else {
        return Err(OpenAiProviderError::UnsupportedOperation);
    };
    match operation {
        OpenAiOperation::Chat(call) => chat(&call, composition, deps.answer, logger).await,
        OpenAiOperation::Embedding(call) => embedding(&call, composition, deps.answer).await,
        OpenAiOperation::Text(call) => text(&call, composition, deps, logger).await,
    }
}

async fn chat<Answer, Logger>(
    call: &ChatCall,
    composition: &Composition,
    answer: &Answer,
    logger: &mut Logger,
) -> Result<OperationResult, OpenAiProviderError>
where
    Answer: ProviderAnswer,
    Logger: OpenAiLogger + Send,
{
    let plan = chat_plan(composition, call)?;
    let upstream = match answer.answer(&plan.prepared).await {
        Ok(upstream) => upstream,
        Err(error) => return Err(logged_failure(logger, call, &plan, error)),
    };
    let result = match OpenAi.decode(&plan.prepared, upstream) {
        Ok(result) => result,
        Err(error) => return Err(logged_failure(logger, call, &plan, error)),
    };
    if call.request.stream {
        logger.post_call(LoggedCall {
            input: logged_json(&call.request.messages),
            // LiteLLM logs the opened stream itself, which betterlite hands to the client unrendered
            original_response: Json::Null,
            headers: plan.logged_headers.as_ref().map(logged_headers),
        });
    }
    Ok(result)
}

// main.py catches every execution failure, logs the error string beside the resolved headers, and rethrows it unchanged.
fn logged_failure<Logger>(
    logger: &mut Logger,
    call: &ChatCall,
    plan: &ChatPlan,
    error: OperationError,
) -> OpenAiProviderError
where
    Logger: OpenAiLogger,
{
    logger.post_call(LoggedCall {
        input: logged_json(&call.request.messages),
        original_response: Json::Text(error.to_string()),
        headers: plan.logged_headers.as_ref().map(logged_headers),
    });
    OpenAiProviderError::Operation(error)
}

async fn embedding<Answer>(
    call: &EmbeddingCall,
    composition: &Composition,
    answer: &Answer,
) -> Result<OperationResult, OpenAiProviderError>
where
    Answer: ProviderAnswer,
{
    let prepared = embedding_plan(composition, call)?;
    let upstream = answer.answer(&prepared).await?;
    Ok(OpenAi.decode(&prepared, upstream)?)
}

async fn text<Answer, Source, Logger>(
    call: &TextCall,
    composition: &Composition,
    deps: Deps<'_, Answer, Source>,
    logger: &mut Logger,
) -> Result<OperationResult, OpenAiProviderError>
where
    Source: TextClientSource,
    <Source::Constructor as ClientConstructor>::Client: Clone,
    Logger: OpenAiLogger + Send,
{
    let plan = text::plan(composition, call)?;
    let outcome = text::execute(&plan, deps.text, deps.now, deps.next_id, logger).await?;
    let response = match outcome {
        TextOutcome::Buffered(response) => response,
        TextOutcome::Stream(stream) => {
            logger.post_call(LoggedCall {
                input: logged_json(&plan.messages),
                // the opened text stream has no rendered JSON form, so the log records its absence
                original_response: Json::Null,
                headers: plan.headers.as_ref().map(logged_headers),
            });
            return Ok(OperationResult::OpenAi(OpenAiResult::TextStream(stream)));
        }
    };
    let now = (deps.now)();
    let (result, reply_log) = buffered_text(&plan, response, &now, deps.next_id)?;
    // main.py logs a truthy stream option or an async call; the transport refused every stream value but a bool
    if matches!(
        plan.optional_params.get("stream"),
        Some(serde_json::Value::Bool(true))
    ) || plan.acompletion
    {
        logger.post_call(LoggedCall {
            input: logged_json(&plan.messages),
            original_response: reply_log,
            headers: plan.headers.as_ref().map(logged_headers),
        });
    }
    Ok(result)
}

// main.py converts the buffered reply to chat for a literally false stream and both flags, and logs what it returns.
fn buffered_text(
    plan: &text::TextPlan,
    response: JsonValue,
    now: &StreamInstant,
    next_id: fn() -> Uuid,
) -> Result<(OperationResult, Json), OpenAiProviderError> {
    if text::projects_to_chat(plan) {
        let chat = text::project(&response, now, next_id)?;
        let logged = logged_json(&chat);
        Ok((
            OperationResult::Chat(ChatCompletion::OpenAi(Box::new(chat))),
            logged,
        ))
    } else {
        let logged = logged_json(&response);
        Ok((
            OperationResult::OpenAi(OpenAiResult::Text(Box::new(response))),
            logged,
        ))
    }
}

// main.py logs the resolved supplied header map itself, never the extra headers the SDK merged beside it
fn logged_headers(headers: &Headers) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}
