use betterlite_models::anthropic_pricing::error::AnthropicPricingError;
use thiserror::Error;

use super::SearchFormula;

#[derive(Debug, Error)]
pub enum FormulaError {
    #[error("no {0:?} web search formula is installed")]
    WebSearch(SearchFormula),
    #[error("no Google Maps grounding formula is installed")]
    GoogleMaps,
    #[error("the Anthropic web search cost could not be computed")]
    Anthropic {
        #[source]
        source: AnthropicPricingError,
    },
}
