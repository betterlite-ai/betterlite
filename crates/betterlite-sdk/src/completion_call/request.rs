use super::error::CompletionCallError;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::{CallOutcome, ChatStream};
use crate::call_wrapper::ports::IdSource;
use crate::completion_request::args::CompletionArgs;
use crate::completion_request::call::{PreparedCall, PreparedTimeout};
use betterlite_context::call_context::{CallContext, CallInputs};
use betterlite_core::dispatch::Dispatch;
use betterlite_core::operation::chat::first_message::first_message_has_content;
use betterlite_core::operation::chat::{
    ChatCall, ChatRequest as DispatchChatRequest, ChatResult, ChatStreamAnswer,
};
use betterlite_core::operation::{CallResult, Operation};
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::stream::SseParser;
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use uuid::Uuid;
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
    ids: &dyn IdSource,
) -> Result<Operation, CompletionCallError> {
    let request = ChatRequest::parse(body)?;
    Ok(Operation::Chat(DispatchChatRequest {
        call_id: call_id(call, ids),
        model_id: call.model.clone(),
        request: Box::new(request),
        first_message_has_content: first_message_has_content(body),
        calls,
    }))
}
#[must_use]
pub fn call_id(call: &PreparedCall, ids: &dyn IdSource) -> String {
    match call.kwargs.get("litellm_call_id") {
        Some(Json::Text(id)) => id.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => ids.next_call_id(),
    }
}
pub(crate) fn timeout_duration(timeout: &PreparedTimeout) -> Option<Duration> {
    match timeout {
        PreparedTimeout::Seconds(seconds) => Duration::try_from_secs_f64(*seconds).ok(),
        PreparedTimeout::Phases(phases) => {
            let parts = [phases.connect, phases.read, phases.write, phases.pool];
            if parts.iter().all(Option::is_none) {
                return None;
            }
            let total: f64 = parts.into_iter().flatten().sum();
            Duration::try_from_secs_f64(total).ok()
        }
    }
}
pub async fn execute(
    dispatch: &Dispatch,
    operation: Operation,
    timeout: Option<&PreparedTimeout>,
    body_bytes: usize,
) -> Result<CallOutcome, CompletionCallError> {
    let started = SystemTime::now();
    let context = CallContext::start(CallInputs {
        now: || started,
        id: Uuid::new_v4,
        deadline: timeout
            .and_then(timeout_duration)
            .and_then(|wait| started.checked_add(wait)),
        privacy: false,
    });
    let result = dispatch.execute(operation, &context).await?;
    outcome_of(result, body_bytes).await
}
async fn outcome_of(
    result: CallResult,
    body_bytes: usize,
) -> Result<CallOutcome, CompletionCallError> {
    let CallResult::Chat(chat) = result else {
        return Err(CompletionCallError::UnexpectedResult { entry: "non-chat" });
    };
    match chat {
        ChatResult::Completion(answer) => {
            Ok(CallOutcome::Response(serde_json::from_slice(&answer.body)?))
        }
        ChatResult::Stream(answer) => stream_outcome(answer, body_bytes).await,
    }
}
async fn stream_outcome(
    answer: ChatStreamAnswer,
    body_bytes: usize,
) -> Result<CallOutcome, CompletionCallError> {
    let mut body = answer.body;
    let mut parser = SseParser::default();
    let mut chunks = Vec::new();
    let mut received = 0_usize;
    while let Some(item) = body.next().await {
        let bytes = item?;
        received = received.saturating_add(bytes.len());
        if received > body_bytes {
            return Err(CompletionCallError::Http(HttpError::TooLarge {
                limit: body_bytes,
            }));
        }
        let events = parser.push(&bytes).map_err(|_error| {
            CompletionCallError::Http(HttpError::TooLarge { limit: body_bytes })
        })?;
        append_payloads(&mut chunks, &events)?;
    }
    let rest = parser.finish();
    append_payloads(&mut chunks, &rest)?;
    Ok(CallOutcome::ChatStream(ChatStream {
        model_response: Json::List(chunks),
        json_mode: false,
        converted_from_mock: false,
    }))
}
fn append_payloads(
    chunks: &mut Vec<Json>,
    payloads: &[Vec<u8>],
) -> Result<(), CompletionCallError> {
    for payload in payloads {
        let data = payload.trim_ascii();
        if data.is_empty() || data == b"[DONE]" {
            continue;
        }
        chunks.push(serde_json::from_slice(data)?);
    }
    Ok(())
}
