use std::error::Error;

#[derive(Debug, thiserror::Error)]
pub enum SearchCostError<PricingError>
where
    PricingError: Error + 'static,
{
    #[error("the pricing lookup for {model} failed: {source}")]
    ModelInfo {
        model: String,
        #[source]
        source: PricingError,
    },
    #[error("the search cost arithmetic overflowed a decimal")]
    Overflow,
}
