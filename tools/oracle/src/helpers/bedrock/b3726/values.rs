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

pub(super) fn encode<T: Serialize>(value: &T) -> Option<GuardrailValue> {
    serde_json::from_value(serde_json::to_value(value).ok()?).ok()
}

pub(super) fn decode<T: DeserializeOwned>(value: &GuardrailValue) -> Option<T> {
    serde_json::from_value(serde_json::to_value(value).ok()?).ok()
}

pub(super) fn to_wire(messages: &[GuardrailValue]) -> Result<Vec<Message>, HookFailure> {
    encode(messages).ok_or_else(|| failed("the guardrail messages could not be decoded"))
}

pub(super) fn to_values(messages: &[Message]) -> Option<Vec<GuardrailValue>> {
    encode(messages)
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
        GuardrailFailure::ModifyResponse(failure) => {
            HookFailure::ModifyResponse(Box::new(ModifyFailure {
                message: failure.message,
                model: Some(failure.model),
                original_response: failure.original_response,
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
