use betterlite_core::api::request_guardrail_policy::error::RequestGuardrailPolicyError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HookPayloadError {
    #[error("the tool call arguments could not be written as JSON")]
    Arguments(#[source] serde_json::Error),
    #[error("user_api_key_auth.{field} must be an object whose {key} is a list of strings")]
    AuthMetadata {
        field: &'static str,
        key: &'static str,
    },
    #[error("metadata.guardrails must be a list")]
    GuardrailList,
    #[error(transparent)]
    Guardrails(#[from] RequestGuardrailPolicyError),
}
