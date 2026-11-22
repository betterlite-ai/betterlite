use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use serde::Deserialize;

use crate::api::bfl::poll::PollSleeper;
use crate::api::responses_session_history::error::SessionHistoryError;
use crate::api::responses_session_history::query::session_messages;
use crate::api::responses_session_history::{
    ChatSession, ColdStorageReader, SessionHistory, SpendLogQuery,
};
use crate::api::responses_tool_repair::repair_tool_results;
use crate::api::responses_tool_repair::value::Value;

// the litellm_completion_request fields the session hook reads
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CompletionRequest {
    #[serde(default)]
    pub messages: Vec<Value>,
    #[serde(default)]
    pub tools: Vec<Value>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub custom_llm_provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedSession {
    pub messages: Vec<Value>,
    pub litellm_trace_id: Option<String>,
}

// the recovered history goes before the new messages, and the session id becomes the trace id
pub async fn merge_session<Query, Cold, Sleep>(
    service: &SessionHistory<'_, Query, Cold, Sleep>,
    previous_response_id: &str,
    request: &CompletionRequest,
) -> Result<MergedSession, SessionHistoryError<Cold::Error>>
where
    Query: SpendLogQuery + Sync,
    Cold: ColdStorageReader + Sync,
    Sleep: PollSleeper + Sync,
{
    let session = if previous_response_id.is_empty() {
        ChatSession::default()
    } else {
        session_messages(service, previous_response_id).await?
    };
    let mut combined = session.messages.clone();
    combined.extend(request.messages.iter().cloned());
    let repaired = repair_tool_results(&combined, &request.tools, service.tool_calls)?;
    // a repair that drops every message falls back to what came in, the new turn first
    let messages = match (
        repaired.is_empty(),
        request.messages.is_empty(),
        session.messages.is_empty(),
    ) {
        (false, _, _) => repaired,
        (true, false, _) => request.messages.clone(),
        (true, true, false) => session.messages,
        (true, true, true) => {
            return Err(SessionHistoryError::NoMessages(no_messages(
                previous_response_id,
                request,
            )));
        }
    };
    Ok(MergedSession {
        messages,
        litellm_trace_id: session.litellm_session_id,
    })
}

fn no_messages(previous_response_id: &str, request: &CompletionRequest) -> ErrorClass {
    let message = format!(
        "Unable to create messages for completion request. This can happen when: 1) Using previous_response_id without a session database, AND 2) Input contains only function_call_output with empty or invalid call_id. Please ensure function_call_output has a valid call_id from a previous response. Original request: previous_response_id={previous_response_id}"
    );
    let context = ErrorContext {
        message,
        model: Some(request.model.clone()),
        provider: Some(request.custom_llm_provider.clone()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, None)))
}
