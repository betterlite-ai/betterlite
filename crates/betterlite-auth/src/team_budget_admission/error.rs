use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException; // ast-grep-ignore: no-long-names
use thiserror::Error;

use crate::auth_end_users::defaults::MemberBudgetError;
use crate::auth_key_budgets::key_budget::Denial;
use crate::auth_key_budgets::spend::SpendReadError;
use crate::membership_load::error::MembershipLoadError;

#[derive(Debug, Error)]
pub enum TeamBudgetError {
    #[error(transparent)]
    Denied(#[from] Denial),
    #[error(transparent)]
    Spend(#[from] SpendReadError),
    #[error(transparent)]
    Membership(#[from] MembershipLoadError),
    #[error(transparent)]
    DefaultBudget(#[from] MemberBudgetError),
    // the client sees B747's safe message; the internal message names the user, team and allowlist for the failure log
    #[error("the team member's model scope does not allow the requested model")]
    // ast-grep-ignore: no-long-names
    ModelDenied(Box<ModelAccessDeniedProxyException>),
    // _can_object_call_model raises a plain Exception here, which the member scope does not catch
    #[error("Unable to parse model, max fallback depth exceeded - received model: {model}")]
    RecurseDepth { model: String },
}
