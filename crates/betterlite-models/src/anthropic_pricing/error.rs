use thiserror::Error;

use crate::token_cost::error::TokenCostError;

#[derive(Debug, Error)]
pub enum AnthropicPricingError {
    #[error(transparent)]
    Token(#[from] TokenCostError),
    #[error("the web search request count must be a JSON number or boolean, not {kind}")]
    InvalidSearchCount { kind: &'static str },
    #[error("anthropic price arithmetic overflowed")]
    Overflow,
}
