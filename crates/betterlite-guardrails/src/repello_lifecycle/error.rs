use std::time::TryFromFloatSecsError;

use betterlite_redact::error::RedactionError;
use thiserror::Error;

use crate::guardrail_base::error::GuardrailBaseError;

#[derive(Debug, Error)]
pub enum RepelloLifecycleError {
    #[error(
        "Couldn't get Repello API key. Set `ARGUS_API_KEY` in the environment or pass `api_key` to the guardrail in the config file."
    )]
    MissingApiKey,
    #[error(
        "Repello guardrail requires an `asset_id`. Create an asset in the Repello dashboard and set `asset_id` on the guardrail in the config file."
    )]
    MissingAssetId,
    #[error("the `{field}` parameter of the RepelloAI Argus guardrail has the wrong type")]
    InvalidParam { field: &'static str },
    #[error("the RepelloAI Argus event hook `{hook}` is not a guardrail event hook")]
    InvalidEventHook { hook: String },
    #[error("the RepelloAI Argus timeout {seconds} is not a representable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the RepelloAI Argus HTTP client could not be built")]
    Client {
        #[source]
        source: reqwest::Error,
    },
    #[error("the RepelloAI Argus redactor could not be built")]
    Redactor {
        #[source]
        source: RedactionError,
    },
    #[error(transparent)]
    Base(#[from] GuardrailBaseError),
}
