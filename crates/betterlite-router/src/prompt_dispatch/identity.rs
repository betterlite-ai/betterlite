use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use thiserror::Error;

pub const LITELLM_AGENT_PREFIX: &str = "litellm_agent/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptIdentity {
    pub prompt_id: Option<Json>,
    pub prompt_variables: Option<Json>,
    pub prompt_label: Option<Json>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PromptIdentityError {
    #[error("prompt_id must be a string")]
    PromptId,
    #[error("prompt_variables must be an object")]
    PromptVariables,
}

// The identity Router._prompt_management_factory resolves before it compiles the prompt.
pub fn resolve_prompt_identity(
    request: &JsonObject,
    data: &JsonObject,
) -> Result<PromptIdentity, PromptIdentityError> {
    let prompt_id = prefer(request.get("prompt_id"), data.get("prompt_id"));
    let prompt_variables = prefer(
        request.get("prompt_variables"),
        data.get("prompt_variables"),
    );
    let prompt_label = prefer(request.get("prompt_label"), data.get("prompt_label"));
    let agent_model = matches!(
        data.get("model"),
        Some(Json::Text(model)) if model.starts_with(LITELLM_AGENT_PREFIX)
    );
    if !agent_model && !matches!(prompt_id.as_ref(), Some(Json::Text(_))) {
        return Err(PromptIdentityError::PromptId);
    }
    if prompt_variables
        .as_ref()
        .is_some_and(|value| !matches!(value, Json::Object(_)))
    {
        return Err(PromptIdentityError::PromptVariables);
    }
    Ok(PromptIdentity {
        prompt_id,
        prompt_variables,
        prompt_label,
    })
}

// A request value wins when it is set; an absent or null key takes the deployment's value.
fn prefer<'value>(request: Option<&'value Json>, data: Option<&'value Json>) -> Option<Json> {
    request
        .filter(|value| !matches!(value, Json::Null))
        .cloned()
        .or_else(|| data.cloned())
}
