use betterlite_models::token_cost::error::TokenCostError;
use thiserror::Error;

// The two failures a cost lookup reports for a relayed response: the catalog has no entry for the model, or the model's entry cannot charge the response.
#[derive(Debug, Error)]
pub enum CostError {
    #[error("the model is not registered in the pricing catalog")]
    UnknownModel(#[source] TokenCostError),
    #[error("the model's price record cannot charge this response")]
    UnpricedField(#[source] TokenCostError),
}
