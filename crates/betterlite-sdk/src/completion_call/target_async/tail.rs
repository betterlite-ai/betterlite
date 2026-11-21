use super::AsyncCompletionTarget;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::{CallOutcome, ChatStream};
use crate::call_wrapper::error::{CallError, plain_error};
use crate::call_wrapper::target::CallTarget as _;
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
use futures_util::{StreamExt as _, stream, stream::BoxStream};
use indexmap::IndexMap;
use serde_json::value::{RawValue, to_raw_value};
pub(super) async fn agentic(
    target: &AsyncCompletionTarget<'_>,
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
    let raw = RawValue::from_string(text)?;
    let model = args.model.clone().unwrap_or_default();
    let messages = followup_messages(&args.messages)?;
    let optional = optional_params(args)?;
    let kwargs = raw_map(&args.kwargs)?;
    let call = ChatLoopCall {
        response: raw.as_ref(),
        model: &model,
        messages: &messages,
        optional_params: &optional,
        kwargs: &kwargs,
        logging: AgenticLoggingHandle::Absent,
        custom_llm_provider: provider.unwrap_or(""),
        stream: args.stream == Some(true),
    };
    let ports = LoopPorts {
        continuation: target,
        fake_stream: target.fake_stream,
    };
    let Some(result) = maybe_run_chat_agentic_loop(&call, target.callbacks, &ports).await? else {
        return Ok(outcome);
    };
    match result {
        ChatLoopResult::Response(answer) => {
            Ok(CallOutcome::Response(serde_json::from_str(answer.get())?))
        }
        ChatLoopResult::Stream(stream) => chunk_outcome(stream).await,
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
                serde_json::from_str::<Json>(value.get()).map_err(|error| {
                    self.async_error(&CompletionCallError::from(error), "", None)
                })?,
            );
        }
        let mut list = Vec::with_capacity(messages.len());
        for message in messages {
            list.push(
                serde_json::from_str::<Json>(message.0.get()).map_err(|error| {
                    self.async_error(&CompletionCallError::from(error), "", None)
                })?,
            );
        }
        request.insert(String::from("model"), Json::Text(model.to_owned()));
        request.insert(String::from("messages"), Json::List(list));
        match self.call(request, None).await? {
            CallOutcome::Response(json) | CallOutcome::ResponsesStream(json) => {
                Ok(ChatLoopResult::Response(to_raw_value(&json).map_err(
                    |source| CallError::from(plain_error(&source.to_string())),
                )?))
            }
            CallOutcome::ChatStream(stream) => {
                let mut chunks = Vec::new();
                if let Json::List(items) = stream.model_response {
                    for item in items {
                        let encoded = serde_json::to_value(item).map_err(|error| {
                            self.async_error(&CompletionCallError::from(error), "", None)
                        })?;
                        chunks.push(serde_json::from_value(encoded).map_err(|error| {
                            self.async_error(&CompletionCallError::from(error), "", None)
                        })?);
                    }
                }
                Ok(ChatLoopResult::Stream(stream::iter(chunks).boxed()))
            }
        }
    }
}
fn followup_messages(messages: &[Json]) -> Result<Vec<FollowupMessage>, CompletionCallError> {
    messages
        .iter()
        .map(|message| Ok(FollowupMessage(to_raw_value(message)?)))
        .collect()
}
fn raw_map(values: &IndexMap<String, Json>) -> Result<JsonMap, CompletionCallError> {
    values
        .iter()
        .map(|(name, value)| Ok((name.clone(), to_raw_value(value)?)))
        .collect()
}
fn optional_params(args: &CompletionArgs) -> Result<JsonMap, CompletionCallError> {
    let encoded = serde_json::to_value(args)?;
    let fields: IndexMap<String, Json> = serde_json::from_value(encoded)?;
    fields
        .into_iter()
        .filter(|(name, value)| {
            !matches!(
                name.as_str(),
                "model" | "messages" | "stream" | "acompletion" | "deployment_id"
            ) && !matches!(value, Json::Null)
        })
        .map(|(name, value)| Ok((name, to_raw_value(&value)?)))
        .collect()
}
pub(crate) async fn chunk_outcome(
    stream: BoxStream<'static, Chunk>,
) -> Result<CallOutcome, CompletionCallError> {
    let chunks: Vec<Chunk> = stream.collect().await;
    Ok(CallOutcome::ChatStream(ChatStream {
        model_response: serde_json::from_value(serde_json::to_value(&chunks)?)?,
        json_mode: false,
        converted_from_mock: false,
    }))
}
