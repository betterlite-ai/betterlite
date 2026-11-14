mod tests;

use std::error::Error;
use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::stream::chunk::Chunk;
use futures_util::stream::BoxStream;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::genai_chat_input::input::{GenerateContentRequest, GenerateLiteLLMParams};
use crate::api::genai_chat_input::{
    GenAiInputError, TranslationContext, translate_generate_content,
};
use crate::api::genai_chat_output::{
    GenAiOutputError, GenerateContentResponse, translate_completion, translate_stream,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum Operation {
    Completion,
    Acompletion,
}

// the caller's kwargs that travel to the completion call next to the translated request
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForwardedFields {
    pub metadata: Option<IndexMap<String, Json>>,
    pub extra_headers: Option<IndexMap<String, String>>,
    pub proxy_server_request: Option<IndexMap<String, Json>>,
}

pub struct GenerateContentCall<'call> {
    pub request: &'call GenerateContentRequest,
    pub litellm_params: &'call GenerateLiteLLMParams,
    pub context: &'call TranslationContext<'call>,
    pub stream: bool,
    pub forwarded: ForwardedFields,
}

// GenerateContentCompletionKwargs: the prepared request plus the fields the translation does not carry
#[derive(Debug, Clone)]
pub struct CompletionCall {
    pub request: ChatRequest,
    pub litellm_params: Option<GenerateLiteLLMParams>,
    pub forwarded: ForwardedFields,
}

pub enum CompletionOutput {
    Response(Box<ChatResponse>),
    Stream(BoxStream<'static, Chunk>),
}

// B014's completion pipeline sits behind this seam, so the handler never names a provider
pub trait CompletionExecutor {
    type Error: Error + Send + 'static;

    fn completion(&self, call: CompletionCall) -> Result<CompletionOutput, Self::Error>;

    fn acompletion(
        &self,
        call: CompletionCall,
    ) -> impl Future<Output = Result<CompletionOutput, Self::Error>> + Send;
}

pub enum GenerateContentOutcome {
    Response(GenerateContentResponse),
    Stream(BoxStream<'static, Result<Vec<u8>, GenAiOutputError>>),
}

// the sync handler answers with the result, or with the pending async handler when _is_async is set
pub enum Handled<Pending> {
    Done(GenerateContentOutcome),
    Pending(Pending),
}

// LiteLLM wraps everything after the preparation as ValueError("Error calling litellm.<operation> for generate_content: <error>")
#[derive(Debug, Error)]
pub enum ChatCallError<Source>
where
    Source: Error + 'static,
{
    #[error(transparent)]
    Input(#[from] GenAiInputError),
    #[error("Error calling litellm.{operation} for generate_content: {source}")]
    Completion {
        operation: Operation,
        #[source]
        source: Source,
    },
    #[error("Error calling litellm.{operation} for generate_content: {source}")]
    Output {
        operation: Operation,
        #[source]
        source: GenAiOutputError,
    },
    #[error(
        "Error calling litellm.{operation} for generate_content: the completion answered a stream to a call that did not ask for one"
    )]
    UnexpectedStream { operation: Operation },
}

pub type GenerateContentResult<Source>
where
    Source: Error + 'static,
= Result<GenerateContentOutcome, ChatCallError<Source>>;

pub fn generate_content_handler<'call, Executor>(
    executor: &'call Executor,
    call: GenerateContentCall<'call>,
    is_async: bool,
) -> Result<
    Handled<impl Future<Output = GenerateContentResult<Executor::Error>>>,
    ChatCallError<Executor::Error>,
>
where
    Executor: CompletionExecutor + Sync,
{
    if is_async {
        return Ok(Handled::Pending(async_generate_content_handler(
            executor, call,
        )));
    }
    let stream = call.stream;
    let prepared = prepare_completion_call(call)?;
    let output = executor
        .completion(prepared)
        .map_err(|source| ChatCallError::Completion {
            operation: Operation::Completion,
            source,
        })?;
    finish(Operation::Completion, stream, output).map(Handled::Done)
}

pub async fn async_generate_content_handler<Executor>(
    executor: &Executor,
    call: GenerateContentCall<'_>,
) -> Result<GenerateContentOutcome, ChatCallError<Executor::Error>>
where
    Executor: CompletionExecutor + Sync,
{
    let stream = call.stream;
    let prepared = prepare_completion_call(call)?;
    let output =
        executor
            .acompletion(prepared)
            .await
            .map_err(|source| ChatCallError::Completion {
                operation: Operation::Acompletion,
                source,
            })?;
    finish(Operation::Acompletion, stream, output)
}

fn prepare_completion_call(
    call: GenerateContentCall<'_>,
) -> Result<CompletionCall, GenAiInputError> {
    let (mut request, litellm_params) =
        translate_generate_content(call.request, Some(call.litellm_params), call.context)?;
    request.stream = call.stream;
    Ok(CompletionCall {
        request,
        litellm_params,
        forwarded: call.forwarded,
    })
}

// a stream answer to a streaming call goes to the streaming translation; every other response is translated whole
fn finish<Source>(
    operation: Operation,
    stream: bool,
    output: CompletionOutput,
) -> Result<GenerateContentOutcome, ChatCallError<Source>>
where
    Source: Error + 'static,
{
    match output {
        CompletionOutput::Stream(chunks) if stream => {
            Ok(GenerateContentOutcome::Stream(translate_stream(chunks)))
        }
        CompletionOutput::Stream(_) => Err(ChatCallError::UnexpectedStream { operation }),
        CompletionOutput::Response(response) => translate_completion(&response)
            .map(GenerateContentOutcome::Response)
            .map_err(|source| ChatCallError::Output { operation, source }),
    }
}
