use tracing::debug;

use crate::resolved_key::auth::UserAPIKeyAuth;

use super::alerts::{BudgetAlertKind, BudgetAlertSink, KeyAlertFields, schedule};
use super::key_budget::KeyBudgets;
use super::money::Money;

// _virtual_key_soft_budget_check: token.spend, not the cross-pod reader, is compared with a nonzero soft budget, and crossing it only alerts
pub fn virtual_key_soft_budget_check(
    auth: &UserAPIKeyAuth,
    budgets: &KeyBudgets,
    user_email: Option<&str>,
    alerts: Option<&dyn BudgetAlertSink>,
) {
    let Some(soft_budget) = budgets.soft_budget else {
        return;
    };
    if matches!(soft_budget, Money::Finite(value) if value.is_zero()) {
        return;
    }
    let spend = auth.view.token.spend;
    if !Money::spend_at_least(spend, soft_budget) {
        return;
    }
    debug!(spend = %spend, soft_budget = ?soft_budget, "Crossed Soft Budget for token");
    let fields = KeyAlertFields {
        auth,
        max_budget: budgets.max_budget,
        soft_budget: budgets.soft_budget,
        user_email,
        team_alias: auth.view.team_alias.as_deref(),
    };
    schedule(
        alerts,
        fields.build(BudgetAlertKind::SoftBudget, spend, None),
    );
}
