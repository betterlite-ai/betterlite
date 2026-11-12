use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use super::hooks::DeploymentHook;
use crate::call_setup::RequestMap;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GuardrailError {
    #[error("the {field} value has the wrong type")]
    FieldType { field: &'static str },
}

pub fn get_request_guardrails(request: &RequestMap) -> Result<Vec<String>, GuardrailError> {
    let Some(metadata) = object(request.get("metadata"), "metadata")? else {
        return Ok(Vec::new());
    };
    let Some(requester) = object(
        metadata.get("requester_metadata"),
        "metadata.requester_metadata",
    )?
    else {
        return Ok(Vec::new());
    };
    let Some(value) = requester.get("guardrails") else {
        return Ok(Vec::new());
    };
    let Json::List(values) = value else {
        return match value {
            Json::Null => Ok(Vec::new()),
            Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
                Err(GuardrailError::FieldType {
                    field: "metadata.requester_metadata.guardrails",
                })
            }
        };
    };
    values
        .iter()
        .map(|entry| match entry {
            Json::Text(name) => Ok(name.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                Err(GuardrailError::FieldType {
                    field: "metadata.requester_metadata.guardrails",
                })
            }
        })
        .collect()
}

pub fn get_applied_guardrails(
    request: &RequestMap,
    hooks: &[Arc<dyn DeploymentHook>],
) -> Result<Vec<String>, GuardrailError> {
    let requested = get_request_guardrails(request)?;
    let mut applied = Vec::new();
    for hook in hooks {
        let Some(guardrail) = hook.guardrail() else {
            continue;
        };
        let Some(name) = guardrail.name else {
            continue;
        };
        if guardrail.default_on || requested.contains(&name) {
            applied.push(name);
        }
    }
    Ok(applied)
}

const fn object<'value>(
    value: Option<&'value Json>,
    field: &'static str,
) -> Result<Option<&'value IndexMap<String, Json>>, GuardrailError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(_) => Err(GuardrailError::FieldType { field }),
    }
}
