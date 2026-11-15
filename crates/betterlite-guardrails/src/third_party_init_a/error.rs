use betterlite_config::sections::guardrail_params::error::GuardrailParamsError;
use thiserror::Error;

use crate::crowdstrike_aidr::error::CrowdStrikeError;

#[derive(Debug, Error)]
pub enum ThirdPartyInitAError {
    #[error("Cisco AI Defense: guardrail_name is required")]
    MissingCiscoName,
    #[error("CrowdStrike AIDR guardrail name is required")]
    MissingCrowdStrikeName,
    #[error("Gray Swan guardrail requires a guardrail_name")]
    MissingGraySwanName,
    #[error(
        "GuardrailsAIException - Please pass the Guardrails AI guard name via 'litellm_params::guard_name'"
    )]
    MissingGuardName,
    #[error("Headroom guardrail requires a guardrail_name")]
    MissingHeadroomName,
    #[error("{hook} is not a valid guardrail event hook")]
    InvalidEventHook { hook: String },
    #[error("the guardrail parameter {name} has an invalid value")]
    InvalidParam { name: String },
    #[error("the guardrail parameters could not be read")]
    Params {
        #[source]
        source: GuardrailParamsError,
    },
    #[error("the CrowdStrike AIDR streaming parameters could not be read")]
    Streaming {
        #[source]
        source: Box<CrowdStrikeError>,
    },
}
