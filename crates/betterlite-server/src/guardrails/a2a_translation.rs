pub mod adapter;
pub mod input;
pub mod output;
pub mod paths;
pub mod stream;

use betterlite_a2a::a2a_text::A2aTextError;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum A2aTranslationError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
    #[error(transparent)]
    Text(#[from] A2aTextError),
    #[error("A2A JSON could not be read: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

mod tests;
