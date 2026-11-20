use super::AsyncCompletionTarget;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::{CallOutcome, ChatStream};
use crate::call_wrapper::error::{CallError, plain_error};
use crate::call_wrapper::target::CallTarget;
use crate::completion_call::error::CompletionCallError;
use crate::completion_request::args::CompletionArgs;
use async_trait::async_trait;
use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
use betterlite_core::api::chat_agentic_loop::entry::maybe_run_chat_agentic_loop;
use betterlite_core::api::chat_agentic_loop::{
    ChatContinuation, ChatLoopCall, ChatLoopResult, LoggingHandle as AgenticLoggingHandle,
    LoopPorts,
};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::stream::chunk::Chunk;
use futures_util::{StreamExt as _, stream::BoxStream};
use indexmap::IndexMap;
impl AsyncCompletionTarget<'_> {
    // the provider-agnostic dispatch point for the chat-completions agentic loop
    pub(super) async fn agentic(
        &self,
        outcome: CallOutcome,
        args: &CompletionArgs,
        provider: Option<&str>,
    ) -> Result<CallOutcome, CompletionCallError> {
        let CallOutcome::Response(json) = &outcome else {
            return Ok(outcome);
        };
        let text = serde_json::to_string(json)?;
        if serde_json::from_str::<ChatResponse>(&text).is_err() {
            return Ok(outcome);
        }
        let raw = serde_json::value::RawValue::from_string(text)?;
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
    let chunks: Vec<Chunk> = stream.collect().await;
    Ok(CallOutcome::ChatStream(ChatStream {
        model_response: serde_json::from_value(serde_json::to_value(&chunks)?)?,
        json_mode: false,
        converted_from_mock: false,
    }))
}
