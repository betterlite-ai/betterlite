use thiserror::Error;

use crate::reservation_settlement::error::SettlementError;
use crate::spend_writes::error::SpendWriteError;

#[derive(Debug, Error)]
pub enum SpendIncrementError {
    #[error(transparent)]
    Write(#[from] SpendWriteError),
    #[error(transparent)]
    Settlement(#[from] SettlementError),
    // LiteLLM's json.loads raising on a budget_limits string: the scope's own increments are lost with the error
    #[error("the budget_limits cached for {cache_key} is not a JSON value")]
    BudgetLimits {
        cache_key: String,
        #[source]
        source: serde_json::Error,
    },
}
