use super::error::ThirdPartyHookError;
use crate::conduct::error::ConductCheckError;
use crate::third_party_init_a::conduct::ConductArgs;

// LiteLLM's stand-in constructor raises when the optional conduct-litellm-guard package is missing
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
