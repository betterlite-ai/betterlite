use thiserror::Error;

use crate::auth_key_budgets::key_budget::Denial;
use crate::auth_key_budgets::spend::SpendReadError;
use crate::auth_tag_budgets::TagBudgetError;

// what a tenant budget check can hand its caller: a denial, an unverifiable counter, or a failed batch read
#[derive(Debug, Error)]
pub enum TenantBudgetError {
    #[error(transparent)]
    Denied(#[from] Denial),
    #[error(transparent)]
    Spend(#[from] SpendReadError),
    #[error(transparent)]
    Tag(#[from] TagBudgetError),
}
