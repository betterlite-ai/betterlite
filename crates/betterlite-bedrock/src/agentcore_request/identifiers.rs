use uuid::Uuid;

use super::error::AgentCoreRequestError;
use crate::model_routes::{ParamMap, ParamValue};

pub const SESSION_HEADER: &str = "X-Amzn-Bedrock-AgentCore-Runtime-Session-Id";
pub const USER_HEADER: &str = "X-Amzn-Bedrock-AgentCore-Runtime-User-Id";

pub fn runtime_session_id(
    optional_params: &ParamMap,
    generated: Uuid,
) -> Result<String, AgentCoreRequestError> {
    let Some(value) = optional_params.get("runtimeSessionId") else {
        return Ok(format!("litellm-session-{generated}"));
    };
    match value {
        ParamValue::Bool(_)
        | ParamValue::Number(_)
        | ParamValue::List(_)
        | ParamValue::Mapping(_) => Err(AgentCoreRequestError::WrongType {
            field: "runtimeSessionId",
        }),
        ParamValue::Null => Ok(format!("litellm-session-{generated}")),
        ParamValue::Text(text) if text.is_empty() => Ok(format!("litellm-session-{generated}")),
        ParamValue::Text(text) => Ok(text.clone()),
    }
}

pub fn runtime_user_id(
    optional_params: &ParamMap,
) -> Result<Option<String>, AgentCoreRequestError> {
    match optional_params.get("runtimeUserId") {
        Some(ParamValue::Text(value)) => Ok(Some(value.clone())),
        Some(
            ParamValue::Bool(_)
            | ParamValue::Number(_)
            | ParamValue::List(_)
            | ParamValue::Mapping(_),
        ) => Err(AgentCoreRequestError::WrongType {
            field: "runtimeUserId",
        }),
        None | Some(ParamValue::Null) => Ok(None),
    }
}
