use super::error::CompletionCallError;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::{CallOutcome, ChatStream};
use crate::completion_request::args::CompletionArgs;
use crate::completion_request::call::PreparedCall;
use betterlite_context::call_context::{CallContext, CallInputs};
use betterlite_core::dispatch::Dispatch;
use betterlite_core::operation::chat::first_message::first_message_has_content;
use betterlite_core::operation::chat::{
    ChatCall, ChatRequest as DispatchChatRequest, ChatResult, ChatStreamAnswer,
};
use betterlite_core::operation::{CallResult, Operation};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use std::sync::Arc;
use std::time::SystemTime;
use uuid::Uuid;
// The call's kwargs as the map completion() was handed, with the secret the struct never serializes
pub fn args_to_map(args: &CompletionArgs) -> Result<RequestMap, CompletionCallError> {
    let encoded = serde_json::to_value(args)?;
    let mut map: RequestMap = serde_json::from_value(encoded)?;
    if let Some(key) = &args.api_key {
        map.insert(
            String::from("api_key"),
            Json::Text(key.expose_secret().to_owned()),
        );
    }
    Ok(map)
}
pub fn chat_body(call: &PreparedCall) -> Result<Vec<u8>, CompletionCallError> {
    let mut fields = IndexMap::new();
    for (name, value) in &call.options.params {
        let encoded = serde_json::to_value(value)?;
        fields.insert(name.clone(), serde_json::from_value(encoded)?);
    }
    let messages = serde_json::to_value(&call.messages)?;
    fields.insert(String::from("model"), Json::Text(call.model.clone()));
    fields.insert(String::from("messages"), serde_json::from_value(messages)?);
    fields.insert(String::from("stream"), Json::Bool(call.stream));
    Ok(serde_json::to_vec(&Json::Object(fields))?)
}
pub fn chat_operation(
    call: &PreparedCall,
    body: &[u8],
    calls: Arc<dyn ChatCall>,
) -> Result<Operation, CompletionCallError> {
    let request = ChatRequest::parse(body)?;
    Ok(Operation::Chat(DispatchChatRequest {
        call_id: call_id(call),
        model_id: call.model.clone(),
        request: Box::new(request),
        first_message_has_content: first_message_has_content(body),
        calls,
    }))
}
#[must_use]
pub fn call_id(call: &PreparedCall) -> String {
    match call.kwargs.get("litellm_call_id") {
        Some(Json::Text(id)) => id.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => Uuid::new_v4().to_string(),
    }
}
pub async fn execute(
    dispatch: &Dispatch,
    operation: Operation,
) -> Result<CallOutcome, CompletionCallError> {
    let context = CallContext::start(CallInputs {
        now: SystemTime::now,
        id: Uuid::new_v4,
        deadline: None,
        privacy: false,
    });
    let result = dispatch.execute(operation, &context).await?;
    match result {
        CallResult::Chat(ChatResult::Completion(answer)) => {
            Ok(CallOutcome::Response(serde_json::from_slice(&answer.body)?))
        }
        CallResult::Chat(ChatResult::Stream(answer)) => stream_outcome(answer).await,
        CallResult::Embedding(_)
        | CallResult::TextCompletion(_)
        | CallResult::TextStream(_)
        | CallResult::Moderation(_)
        | CallResult::Responses(_)
        | CallResult::ImageGeneration(_)
        | CallResult::ImageEdit(_)
        | CallResult::VideoGeneration(_)
        | CallResult::VideoList(_)
        | CallResult::VideoStatus(_)
        | CallResult::VideoContent(_)
        | CallResult::VideoRemix(_)
        | CallResult::VideoCreateCharacter(_)
        | CallResult::VideoGetCharacter(_)
        | CallResult::VideoEdit(_)
        | CallResult::VideoExtension(_)
        | CallResult::LlmPassthrough(_)
        | CallResult::ContainerCall(_) => {
            Err(CompletionCallError::UnexpectedResult { entry: "non-chat" })
        }
    }
}
async fn stream_outcome(answer: ChatStreamAnswer) -> Result<CallOutcome, CompletionCallError> {
    let mut body = answer.body;
    let mut bytes = Vec::new();
    while let Some(item) = body.next().await {
        bytes.extend_from_slice(&item?);
    }
    Ok(CallOutcome::ChatStream(ChatStream {
        model_response: sse_chunks(&bytes)?,
        json_mode: false,
        converted_from_mock: false,
    }))
}
pub fn sse_chunks(bytes: &[u8]) -> Result<Json, CompletionCallError> {
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|_error| CompletionCallError::Field { field: "stream" })?;
    let mut chunks = Vec::new();
    for event in text.split("\n\n") {
        for line in event.lines() {
            let Some(field) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = field.trim();
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            chunks.push(serde_json::from_str::<Json>(payload)?);
        }
    }
    Ok(Json::List(chunks))
}
