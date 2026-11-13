use betterlite_foundry::pricing::error::PricingError as FoundryPricingError;
use betterlite_models::anthropic_pricing::error::AnthropicPricingError;
use betterlite_models::billed_tokens::BilledTokenError;
use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::ocr_pricing::OcrCostError;
use betterlite_models::resolution::ResolutionError;
use betterlite_models::tiered_pricing::TieredPricingError;
use betterlite_models::token_cost::error::TokenCostError;
use thiserror::Error;

use super::policy::error::PolicyError;
use crate::cost::batch::error::BatchCostError;

#[derive(Debug, Error)]
pub enum CallPriceError {
    #[error("Invalid arg. Model cannot be none.")]
    NullModel,
    #[error("the {field} count is not a whole number")]
    NonIntegerCount { field: &'static str },
    #[error("the token counts overflow a 64-bit counter")]
    CounterOverflow,
    #[error("a {call_type} call needs its {field} to be priced")]
    MissingInput {
        call_type: &'static str,
        field: &'static str,
    },
    #[error(
        "prompt_characters must be provided for tts calls. model={model}, custom_llm_provider={provider}"
    )]
    MissingPromptCharacters { model: String, provider: String },
    #[error("cost for tts call is None. model={model}, custom_llm_provider={provider}")]
    MissingSpeechCost { model: String, provider: String },
    #[error("resolving the provider failed: {source}")]
    Resolution {
        #[source]
        source: ResolutionError,
    },
    #[error(transparent)]
    Custom(#[from] BilledTokenError),
    #[error(transparent)]
    Generic(#[from] TokenCostError),
    #[error(transparent)]
    Anthropic(#[from] AnthropicPricingError),
    #[error(transparent)]
    Foundry(#[from] FoundryPricingError),
    #[error(transparent)]
    Duration(#[from] DurationCostError),
    #[error(transparent)]
    Price(#[from] TieredPricingError),
    #[error(transparent)]
    Ocr(#[from] OcrCostError),
    #[error(transparent)]
    Batch(#[from] BatchCostError),
    #[error(transparent)]
    Policy(#[from] PolicyError),
}
