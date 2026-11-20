use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::agentic_callback::AgenticCallback;
use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
use betterlite_core::api::chat_agentic_loop::entry::maybe_run_chat_agentic_loop;
use betterlite_core::api::chat_agentic_loop::fake_stream::FakeStreamPort;
use betterlite_core::api::chat_agentic_loop::{
    ChatContinuation, ChatLoopCall, ChatLoopResult, LoggingHandle as AgenticLoggingHandle,
    LoopPorts,
};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::completion_chat_bridge::answer::chat_completion_body;
use betterlite_openai::stream::chunk::Chunk;
use betterlite_openai::text_completion_response::CompletionResponse;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use indexmap::IndexMap;

use super::error::{CompletionCallError, into_call_error};
use super::request;
use super::retry;
use super::target::CompletionTarget;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::{CallOutcome, ChatStream};
use crate::call_wrapper::error::{CallError, plain_error};
use crate::call_wrapper::state::LoggingHandle;
use crate::call_wrapper::target::CallTarget;
use crate::completion_request::args::CompletionArgs;
use crate::completion_request::call::{MockCall, PreparedCompletion};
use crate::completion_request::prepare_acompletion;

const TEXT_COMPLETION_PROVIDERS: [&str; 3] = [
    "text-completion-openai",
    "text-completion-codestral",
    "text-completion-inception",
];

// main.py's acompletion: prepare the async call, then the tail that follows the provider answer
pub struct AsyncCompletionTarget<'ports> {
    pub inner: CompletionTarget<'ports>,
    pub callbacks: &'ports [Arc<dyn AgenticCallback>],
    pub fake_stream: &'ports dyn FakeStreamPort,
}

impl AsyncCompletionTarget<'_> {
    // acompletion's except branch sets custom_llm_provider = custom_llm_provider or "openai"
    fn async_error(
        &self,
        error: CompletionCallError,
        model: &str,
        provider: Option<&str>,
    ) -> CallError {
        into_call_error(
            &error,
            model,
            provider.unwrap_or("openai"),
            &self.inner.redactor,
        )
    }

    // ### APPLY MOCK DELAY ###
    pub(crate) async fn sleep_mock_delay(
        &self,
        call: &MockCall,
    ) -> Result<(), CompletionCallError> {
        let Some(value) = &call.mock_delay else {
            return Ok(());
        };
        let seconds = match value {
            Json::Number(number) => number.as_f64(),
            Json::Text(text) => text.parse::<f64>().ok(),
            Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
        };
        let Some(seconds) = seconds else {
            return Err(CompletionCallError::Field {
                field: "mock_delay",
            });
        };
        let duration =
            Duration::try_from_secs_f64(seconds).map_err(|_error| CompletionCallError::Field {
                field: "mock_delay",
            })?;
        self.inner.sleeper.sleep(duration).await;
        Ok(())
    }

    // a text completion answer from a text-completion provider becomes a chat completion answer
    pub(crate) fn convert_text_reply(
        &self,
        provider: Option<&str>,
        outcome: CallOutcome,
    ) -> Result<CallOutcome, CompletionCallError> {
        let Some(provider) = provider else {
            return Ok(outcome);
        };
        if !TEXT_COMPLETION_PROVIDERS.contains(&provider) {
            return Ok(outcome);
        }
        let parsed = match &outcome {
            CallOutcome::Response(json) => {
                let encoded = serde_json::to_value(json)?;
                serde_json::from_value::<CompletionResponse>(encoded).ok()
            }
            CallOutcome::ChatStream(_) | CallOutcome::ResponsesStream(_) => None,
        };
        let Some(response) = parsed else {
            return Ok(outcome);
        };
        let created = self.inner.clock.now().timestamp();
        let body = chat_completion_body(&response, created, response.model.as_deref())?;
        Ok(CallOutcome::Response(serde_json::from_slice(&body)?))
    }

    // the provider-agnostic dispatch point for the chat-completions agentic loop
    async fn agentic(
        &self,
        outcome: CallOutcome,
        args: &CompletionArgs,
        provider: Option<&str>,
    ) -> Result<CallOutcome, CompletionCallError> {
        let raw = match &outcome {
            CallOutcome::Response(json) => {
                let text = serde_json::to_string(json)?;
                match serde_json::from_str::<ChatResponse>(&text) {
                    Ok(_response) => Some(serde_json::value::RawValue::from_string(text)?),
                    Err(_error) => None,
                }
            }
            CallOutcome::ChatStream(_) | CallOutcome::ResponsesStream(_) => None,
        };
        let Some(raw) = raw else {
            return Ok(outcome);
        };
        let model = args.model.clone().unwrap_or_default();
        let messages = followup_messages(&args.messages)?;
        let optional = optional_params(args)?;
        let kwargs = raw_map(&args.kwargs)?;
        let stream = args.stream == Some(true);
        let call = ChatLoopCall {
            response: raw.as_ref(),
            model: &model,
            messages: &messages,
            optional_params: &optional,
            kwargs: &kwargs,
            logging: AgenticLoggingHandle::Absent,
            custom_llm_provider: provider.unwrap_or(""),
            stream,
        };
        let ports = LoopPorts {
            continuation: self,
            fake_stream: self.fake_stream,
        };
        let Some(result) = maybe_run_chat_agentic_loop(&call, self.callbacks, &ports).await? else {
            return Ok(outcome);
        };
        match result {
            ChatLoopResult::Response(raw) => {
                Ok(CallOutcome::Response(serde_json::from_str(raw.get())?))
            }
            ChatLoopResult::Stream(stream) => chunk_outcome(stream).await,
        }
    }
}

#[async_trait]
impl CallTarget for AsyncCompletionTarget<'_> {
    fn name(&self) -> &str {
        "acompletion"
    }

    async fn call(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        let args =
            request::args_from_map(&request).map_err(|error| self.async_error(error, "", None))?;
        let model = args.model.clone().unwrap_or_default();
        let prepared = prepare_acompletion(&args, &self.inner.settings, &self.inner.ports)
            .await
            .map_err(|error| self.async_error(CompletionCallError::from(error), &model, None))?;
        let provider = provider_of(&prepared).map(str::to_owned);
        let prepared = match prepared {
            PreparedCompletion::Fallbacks(call) => {
                let outcome = super::fallbacks::run_fallbacks(self, call, logging)
                    .await
                    .map_err(|error| self.async_error(error, &model, provider.as_deref()))?;
                if super::fallbacks::is_empty(&outcome) {
                    return Err(self.async_error(
                        CompletionCallError::NoFallbackResponse,
                        &model,
                        provider.as_deref(),
                    ));
                }
                return Ok(outcome);
            }
            PreparedCompletion::Mock(call) => {
                self.sleep_mock_delay(&call)
                    .await
                    .map_err(|error| self.async_error(error, &model, provider.as_deref()))?;
                PreparedCompletion::Mock(call)
            }
            other => other,
        };
        let outcome = self
            .inner
            .run(prepared, true, logging)
            .await
            .map_err(|error| self.async_error(error, &model, provider.as_deref()))?;
        let outcome = self
            .convert_text_reply(provider.as_deref(), outcome)
            .map_err(|error| self.async_error(error, &model, provider.as_deref()))?;
        self.agentic(outcome, &args, provider.as_deref())
            .await
            .map_err(|error| self.async_error(error, &model, provider.as_deref()))
    }

    // acompletion_with_retries defaults original_function to completion, so the blocking entry runs
    async fn retry(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        retry::with_retries(&self.inner, request, logging, self.inner.sleeper).await
    }
}

// the continuation the loop calls to ask for another completion
#[async_trait]
impl ChatContinuation for AsyncCompletionTarget<'_> {
    type Error = CallError;

    async fn complete(
        &self,
        model: &str,
        messages: &[FollowupMessage],
        params: &JsonMap,
        kwargs: &JsonMap,
    ) -> Result<ChatLoopResult, CallError> {
        let mut request = RequestMap::new();
        for (name, value) in params.iter().chain(kwargs.iter()) {
            request.insert(
                name.clone(),
                json_from_raw(value.get()).map_err(|error| self.async_error(error, "", None))?,
            );
        }
        let mut list = Vec::with_capacity(messages.len());
        for message in messages {
            list.push(
                json_from_raw(message.0.get())
                    .map_err(|error| self.async_error(error, "", None))?,
            );
        }
        request.insert(String::from("model"), Json::Text(model.to_owned()));
        request.insert(String::from("messages"), Json::List(list));
        match self.call(request, None).await? {
            CallOutcome::Response(json) => Ok(ChatLoopResult::Response(raw_value(&json)?)),
            CallOutcome::ResponsesStream(json) => Ok(ChatLoopResult::Response(raw_value(&json)?)),
            CallOutcome::ChatStream(stream) => {
                let mut chunks = Vec::new();
                if let Json::List(items) = stream.model_response {
                    for item in items {
                        chunks.push(
                            chunk_of(item).map_err(|error| self.async_error(error, "", None))?,
                        );
                    }
                }
                Ok(ChatLoopResult::Stream(
                    futures_util::stream::iter(chunks).boxed(),
                ))
            }
        }
    }
}

fn provider_of(prepared: &PreparedCompletion) -> Option<&str> {
    match prepared {
        PreparedCompletion::Dispatch(call)
        | PreparedCompletion::ResponsesBridge(call)
        | PreparedCompletion::McpLoop(call) => call.custom_llm_provider.as_deref(),
        PreparedCompletion::Mock(call) => call.call.custom_llm_provider.as_deref(),
        PreparedCompletion::Fallbacks(_) | PreparedCompletion::ModelList(_) => None,
    }
}

fn json_from_raw(raw: &str) -> Result<Json, CompletionCallError> {
    Ok(serde_json::from_str(raw)?)
}

fn raw_value(json: &Json) -> Result<Box<serde_json::value::RawValue>, CallError> {
    serde_json::value::to_raw_value(json)
        .map_err(|source| CallError::from(plain_error(&source.to_string())))
}

fn chunk_of(value: Json) -> Result<Chunk, CompletionCallError> {
    let encoded = serde_json::to_value(value)?;
    Ok(serde_json::from_value(encoded)?)
}

fn followup_messages(messages: &[Json]) -> Result<Vec<FollowupMessage>, CompletionCallError> {
    let mut list = Vec::with_capacity(messages.len());
    for message in messages {
        list.push(FollowupMessage(serde_json::value::to_raw_value(message)?));
    }
    Ok(list)
}

fn raw_map(values: &IndexMap<String, Json>) -> Result<JsonMap, CompletionCallError> {
    let mut map = JsonMap::new();
    for (name, value) in values {
        map.insert(name.clone(), serde_json::value::to_raw_value(value)?);
    }
    Ok(map)
}

fn optional_params(args: &CompletionArgs) -> Result<JsonMap, CompletionCallError> {
    let encoded = serde_json::to_value(args)?;
    let fields: IndexMap<String, Json> = serde_json::from_value(encoded)?;
    let mut map = JsonMap::new();
    for (name, value) in fields {
        if matches!(
            name.as_str(),
            "model" | "messages" | "stream" | "acompletion" | "deployment_id"
        ) {
            continue;
        }
        if matches!(value, Json::Null) {
            continue;
        }
        map.insert(name, serde_json::value::to_raw_value(&value)?);
    }
    Ok(map)
}

async fn chunk_outcome(
    stream: BoxStream<'static, Chunk>,
) -> Result<CallOutcome, CompletionCallError> {
    let mut body = stream;
    let mut chunks = Vec::new();
    while let Some(chunk) = body.next().await {
        let encoded = serde_json::to_value(&chunk)?;
        chunks.push(serde_json::from_value(encoded)?);
    }
    Ok(CallOutcome::ChatStream(ChatStream {
        model_response: Json::List(chunks),
        json_mode: false,
        converted_from_mock: false,
    }))
}
