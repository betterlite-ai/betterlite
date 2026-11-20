use crate::auth_tag_budgets::TagBudgetError;

#[derive(Debug, thiserror::Error)]
pub enum ReservationCounterError {
    #[error("the tag or model access group budgets could not be read")]
    Budgets(#[from] TagBudgetError),
}
