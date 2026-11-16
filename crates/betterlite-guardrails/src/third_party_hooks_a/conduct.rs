use super::error::ThirdPartyHookError;
use crate::conduct::error::ConductCheckError;
use crate::third_party_init_a::conduct::ConductArgs;

// LiteLLM ships a stand-in class whose constructor raises when the optional conduct-litellm-guard
// package is missing or too old for it. betterlite has no such package, so every build fails the
// same way and no Conduct hook can be bound.
#[derive(Debug, Clone)]
pub struct ConductGuardrail {
    pub args: ConductArgs,
}

pub fn build(args: &ConductArgs) -> Result<ConductGuardrail, ThirdPartyHookError> {
    tracing::debug!(
        guardrail = args.name,
        "the Conduct guardrail package is not available"
    );
    Err(ThirdPartyHookError::ConductMissingPackage(
        ConductCheckError::MissingPackage,
    ))
}
