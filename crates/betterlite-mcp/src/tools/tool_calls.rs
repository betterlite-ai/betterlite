use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use std::future::Future;
use tokio::time::timeout;

use super::pagination::{McpSession, Timeouts, ToolError};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct OpenAiToolCall {
    pub function: FunctionCall,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: Option<Json>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct CallRequest {
    pub name: String,
    pub arguments: Option<Object>,
}

#[must_use]
pub fn openai_tool_call_to_mcp(call: &OpenAiToolCall) -> CallRequest {
    let arguments = match &call.function.arguments {
        Some(Json::Text(text)) => serde_json::from_str::<Object>(text).unwrap_or_default(),
        Some(Json::Object(object)) => object.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) | None => Object::new(),
    };
    CallRequest {
        name: call.function.name.clone(),
        arguments: Some(arguments),
    }
}

pub async fn call_mcp_tool<Session>(
    session: &mut Session,
    request: CallRequest,
    timeouts: Timeouts,
) -> Result<Session::Output, ToolError<Session::Error>>
where
    Session: McpSession,
{
    let result = timeout(timeouts.client, session.call_tool(request))
        .await
        .map_err(ToolError::Timeout)?;
    result.map_err(ToolError::Session)
}

pub fn call_openai_tool<'session, Session>(
    session: &'session mut Session,
    call: &OpenAiToolCall,
    timeouts: Timeouts,
) -> impl Future<Output = Result<Session::Output, ToolError<Session::Error>>>
+ 'session
+ use<'session, Session>
where
    Session: McpSession,
{
    let request = openai_tool_call_to_mcp(call);
    call_mcp_tool(session, request, timeouts)
}
