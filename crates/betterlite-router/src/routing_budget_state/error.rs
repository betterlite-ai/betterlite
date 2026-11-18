use thiserror::Error;

use crate::budget_increment::error::IncrementError;

#[derive(Debug, Error)]
pub enum BudgetStateError {
    #[error("the budget spend increment for {key} failed")]
    Increment {
        key: String,
        #[source]
        source: IncrementError,
    },
    #[error("the budget spend queue could not be written at shutdown; {entries} left")]
    Shutdown { entries: usize },
}
