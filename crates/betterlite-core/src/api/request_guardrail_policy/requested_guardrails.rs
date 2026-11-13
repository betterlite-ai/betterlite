use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use crate::api::request_guardrail_policy::error::RequestGuardrailPolicyError;
use crate::api::request_guardrail_policy::metadata::GUARDRAILS;

// LiteLLM moves a requested guardrail unchanged: a name, or a map of guardrail name to dynamic params
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(super) enum RequestedGuardrail {
    Name(String),
    Params(IndexMap<String, DynamicGuardrailParams>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct DynamicGuardrailParams {
    extra_body: IndexMap<String, Json>,
}

pub(super) fn requested_guardrail(
    value: Json,
) -> Result<RequestedGuardrail, RequestGuardrailPolicyError> {
    match value {
        Json::Text(name) => Ok(RequestedGuardrail::Name(name)),
        Json::Object(entries) => {
            let mut params = IndexMap::new();
            for (name, entry) in entries {
                params.insert(name, dynamic_guardrail_params(entry)?);
            }
            Ok(RequestedGuardrail::Params(params))
        }
        Json::Bool(_) | Json::Number(_) | Json::Null | Json::List(_) => {
            Err(RequestGuardrailPolicyError::NotTyped {
                field: GUARDRAILS,
                expected: "a list of guardrail names or guardrail params objects",
            })
        }
    }
}

fn dynamic_guardrail_params(
    value: Json,
) -> Result<DynamicGuardrailParams, RequestGuardrailPolicyError> {
    let Json::Object(mut fields) = value else {
        return Err(RequestGuardrailPolicyError::NotTyped {
            field: GUARDRAILS,
            expected: "a guardrail params object",
        });
    };
    let Some(extra_body) = fields.shift_remove("extra_body") else {
        return Err(RequestGuardrailPolicyError::NotTyped {
            field: GUARDRAILS,
            expected: "a guardrail params object with extra_body",
        });
    };
    if !fields.is_empty() {
        return Err(RequestGuardrailPolicyError::NotTyped {
            field: GUARDRAILS,
            expected: "a guardrail params object holding only extra_body",
        });
    }
    let Json::Object(body) = extra_body else {
        return Err(RequestGuardrailPolicyError::NotTyped {
            field: GUARDRAILS,
            expected: "an extra_body object",
        });
    };
    Ok(DynamicGuardrailParams { extra_body: body })
}
