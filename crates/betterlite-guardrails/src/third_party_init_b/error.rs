use betterlite_config::sections::guardrail_params::error::GuardrailParamsError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ThirdPartyInitBError {
    #[error("PANW Prisma AIRS: guardrail_name is required")]
    MissingPanwName,
    #[error("Pillar guardrail name is required")]
    MissingPillarName,
    #[error("TypeSafe guardrail name is required")]
    MissingTypesafeName,
    #[error("api_key is required for straiker")]
    MissingStraikerApiKey,
    #[error(
        "Straiker guardrail: format_hint {format_hint} must be 'anthropic.messages' or 'openai.chat'"
    )]
    InvalidFormatHint { format_hint: String },
    #[error("TypeSafe guardrail: {hook} is not a guardrail event hook")]
    InvalidEventHook { hook: String },
    #[error("the guardrail parameter {name} has an invalid value")]
    InvalidParam { name: String },
    #[error("the guardrail parameters could not be read")]
    Params {
        #[source]
        source: GuardrailParamsError,
    },
}
