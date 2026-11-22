use uuid::Uuid;

use super::error::InvokeAgentRequestError;
use crate::model_routes::{ParamMap, ParamValue};

// LiteLLM splits the model on "/" and accepts only agent/AGENT_ID/ALIAS_ID, so an empty id passes here
pub fn agent_id_and_alias_id(model: &str) -> Result<(&str, &str), InvokeAgentRequestError> {
    let mut parts = model.split('/');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("agent"), Some(agent_id), Some(alias_id), None) => Ok((agent_id, alias_id)),
        _ => Err(InvokeAgentRequestError::InvalidModelFormat),
    }
}

// optional_params.get("sessionID", None) or str(uuid.uuid4()): every falsey value generates a fresh id
pub fn session_id(
    optional_params: &ParamMap,
    generated: Uuid,
) -> Result<String, InvokeAgentRequestError> {
    match optional_params.get("sessionID") {
        None | Some(ParamValue::Null) => Ok(generated.to_string()),
        Some(ParamValue::Text(text)) if text.is_empty() => Ok(generated.to_string()),
        Some(ParamValue::Text(text)) => Ok(text.clone()),
        Some(_) => Err(InvokeAgentRequestError::WrongType { field: "sessionID" }),
    }
}
