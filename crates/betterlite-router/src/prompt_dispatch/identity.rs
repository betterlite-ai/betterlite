use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use thiserror::Error;

use crate::prompt_dispatch::python_text::{python_text, python_type_name};

pub const LITELLM_AGENT_PREFIX: &str = "litellm_agent/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptIdentity {
    pub prompt_id: Option<Json>,
    pub prompt_variables: Option<Json>,
    pub prompt_label: Option<Json>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PromptIdentityError {
    #[error("Prompt ID is not set or not a string. Got={got}, type=<class '{kind}'>")]
    PromptId { got: String, kind: &'static str },
    #[error("Prompt variables is set but not a dictionary. Got={got}, type=<class '{kind}'>")]
    PromptVariables { got: String, kind: &'static str },
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
        let (got, kind) = match prompt_id.as_ref() {
            Some(value) => (python_text(value), python_type_name(value)),
            None => ("None".to_owned(), "NoneType"),
        };
        return Err(PromptIdentityError::PromptId { got, kind });
    }
    if let Some(value) = prompt_variables
        .as_ref()
        .filter(|value| !matches!(value, Json::Object(_)))
    {
        return Err(PromptIdentityError::PromptVariables {
            got: python_text(value),
            kind: python_type_name(value),
        });
    }
    Ok(PromptIdentity {
        prompt_id,
        prompt_variables,
        prompt_label,
    })
}

// Python's `request.get(key) or data.get(key)`: a request value wins only when it is truthy.
fn prefer<'value>(request: Option<&'value Json>, data: Option<&'value Json>) -> Option<Json> {
    match request {
        Some(value) if is_truthy(value) => Some(value.clone()),
        _ => data.cloned(),
    }
}

fn is_truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number
            .as_f64()
            .is_some_and(|number| number.abs().to_bits() != 0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(entries) => !entries.is_empty(),
    }
}
