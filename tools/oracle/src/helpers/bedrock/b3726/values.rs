use betterlite_bedrock::guardrail_hooks::errors::{HookFailure, ModifyFailure};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::bedrock_message_selection::wire::Message;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub(super) fn failed(message: &str) -> HookFailure {
    HookFailure::Failed {
        message: message.to_owned(),
    }
}

// Converts a typed value to a guardrail value or back, through the JSON form both sides serialize to.
pub(super) fn via_json<A, B>(value: &A) -> Option<B>
where
    A: Serialize + ?Sized,
    B: DeserializeOwned,
{
    let Ok(payload) = serde_json::to_value(value) else {
        return None;
    };
    serde_json::from_value(payload).ok()
}

pub(super) fn to_wire(messages: &[GuardrailValue]) -> Result<Vec<Message>, HookFailure> {
    let Some(wire) = via_json(messages) else {
        return Err(failed("the guardrail messages could not be decoded"));
    };
    Ok(wire)
}

pub(super) fn hook_failure(failure: GuardrailFailure) -> HookFailure {
    match failure {
        GuardrailFailure::HttpObject { status, detail } => HookFailure::Rejected {
            status,
            detail: GuardrailValue::Object(detail.into_iter().collect()),
        },
        GuardrailFailure::Http { status, detail } => HookFailure::Rejected {
            status,
            detail: GuardrailValue::Text(detail),
        },
        GuardrailFailure::ModifyResponse(modify) => {
            HookFailure::ModifyResponse(Box::new(ModifyFailure {
                message: modify.message,
                model: Some(modify.model),
                original_response: modify.original_response,
            }))
        }
        other @ (GuardrailFailure::GuardrailRaised { .. }
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::Other(_)) => HookFailure::Failed {
            message: other.to_string(),
        },
    }
}
