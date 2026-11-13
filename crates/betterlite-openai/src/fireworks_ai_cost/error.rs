use betterlite_models::token_cost::error::TokenCostError;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("the model info lookup failed")]
pub struct LookupFailed;

#[derive(Debug, Error)]
pub enum FireworksCostError {
    #[error("no model info is mapped for {model}")]
    Lookup {
        model: String,
        #[source]
        source: LookupFailed,
    },
    #[error("the model info for {model} has no text litellm_provider")]
    Provider { model: String },
    #[error(transparent)]
    Cost(#[from] TokenCostError),
}
