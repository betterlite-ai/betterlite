use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use indexmap::IndexMap;
use serde::Serialize;
use uuid::Uuid;

use crate::operation::{ChatCompletion, OperationRequest, OperationResult};
use crate::provider::Adapter as _;

use super::call::{ChatCall, Composition, EmbeddingCall, OpenAiOperation, TextCall};
use super::compose::{ChatPlan, chat_plan, embedding_plan};
use super::error::OpenAiProviderError;
use super::logging::{ChatPostCall, OpenAiLogger, TextPostCall, logged_json};
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
    Source::Constructor::Client: Clone,
    Logger: OpenAiLogger,
{
    let OperationRequest::OpenAi(OpenAiRequest) = request else {
        return Err(OpenAiProviderError::UnsupportedOperation);
    };
    match operation {
        OpenAiOperation::Chat(call) => chat(&call, composition, deps.answer, logger).await,
        OpenAiOperation::Embedding(call) => embedding(&call, composition, deps.answer).await,
        OpenAiOperation::Text(call) => {
            text(
                &call,
                composition,
                deps.text,
                deps.now,
                deps.next_id,
                logger,
            )
            .await
        }
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
    Logger: OpenAiLogger,
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
        logger.chat_post_call(ChatPostCall {
            input: logged_input(&call.request.messages),
            // LiteLLM logs the opened stream itself, which betterlite hands to the client unrendered
            original_response: Json::Null,
            headers: logged_headers(plan.headers.as_ref()),
        });
    }
    Ok(result)
}

// main.py catches every execution failure, logs the error string beside the resolved headers, and rethrows it unchanged.
fn logged_failure<Logger: OpenAiLogger>(
    logger: &mut Logger,
    call: &ChatCall,
    plan: &ChatPlan,
    error: OpenAiProviderError,
) -> OpenAiProviderError {
    logger.chat_post_call(ChatPostCall {
        input: logged_input(&call.request.messages),
        original_response: Json::Text(error.to_string()),
        headers: logged_headers(plan.headers.as_ref()),
    });
    error
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
    OpenAi.decode(&prepared, upstream)
}

async fn text<Source, Logger>(
    call: &TextCall,
    composition: &Composition,
    source: &Source,
    now: fn() -> StreamInstant,
    next_id: fn() -> Uuid,
    logger: &mut Logger,
) -> Result<OperationResult, OpenAiProviderError>
where
    Source: TextClientSource,
    Source::Constructor::Client: Clone,
    Logger: OpenAiLogger,
{
    let plan = text::plan(composition, call, now())?;
    let outcome = text::execute(&plan, source, now, next_id, logger).await?;
    let response = match outcome {
        TextOutcome::Buffered(response) => response,
        TextOutcome::Stream(stream) => {
            logger.text_post_call(TextPostCall::new(
                logged_input(&plan.messages),
                Json::Null,
                logged_headers(plan.headers.as_ref()),
            ));
            return Ok(OperationResult::OpenAi(OpenAiResult::TextStream(stream)));
        }
    };
    let (result, logged) = if text::projects_to_chat(&plan) {
        let chat = text::project(&plan, &response)?;
        let logged = logged_json(&chat);
        (
            OperationResult::Chat(ChatCompletion::OpenAi(Box::new(chat))),
            logged,
        )
    } else {
        (
            OperationResult::OpenAi(OpenAiResult::Text(Box::new(response.clone()))),
            logged_json(&response),
        )
    };
    if text::logs_post_call(&plan) {
        logger.text_post_call(TextPostCall::new(
            logged_input(&plan.messages),
            logged,
            logged_headers(plan.headers.as_ref()),
        ));
    }
    Ok(result)
}

fn logged_input<Message: Serialize>(messages: &[Message]) -> Json {
    logged_json(&messages)
}

// main.py logs the resolved supplied header map itself, never the extra headers the SDK merged beside it
fn logged_headers(headers: Option<&Headers>) -> Option<IndexMap<String, Json>> {
    headers.map(|headers| {
        headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect()
    })
}
