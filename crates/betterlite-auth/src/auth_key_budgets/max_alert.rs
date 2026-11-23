use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use tracing::debug;

use crate::resolved_key::auth::UserAPIKeyAuth;

use super::alerts::{BudgetAlertKind, BudgetAlertSink, KeyAlertFields, schedule};
use super::email::{merge_budget_alert_email_configs, min_digit_threshold};
use super::key_budget::KeyBudgets;
use super::money::Money;

// _virtual_key_max_budget_alert_check: a merged threshold map alerts at its lowest digit-only
// percentage, even past the maximum; without a map the legacy fraction alerts only strictly below
// the maximum, and only that branch logs the threshold
pub fn virtual_key_max_budget_alert_check(
    auth: &UserAPIKeyAuth,
    budgets: &KeyBudgets,
    user_email: Option<&str>,
    global_config: Option<&BTreeMap<String, JsonData>>,
    fraction: Money,
    alerts: Option<&dyn BudgetAlertSink>,
) {
    let Some(max_budget) = budgets.max_budget else {
        return;
    };
    let spend = auth.view.token.spend;
    if spend <= Decimal::ZERO {
        return;
    }
    let per_key_config = match auth.view.token.metadata.0.get("max_budget_alert_emails") {
        Some(JsonData::Object(fields)) => Some(fields),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Array(_),
        )
        | None => None,
    };
    let fields = KeyAlertFields {
        auth,
        max_budget: budgets.max_budget,
        soft_budget: budgets.soft_budget,
        user_email,
        team_alias: auth.view.team_alias.as_deref(),
    };
    if let Some(config) = merge_budget_alert_email_configs(global_config, per_key_config) {
        let Some(min_pct) = min_digit_threshold(&config) else {
            return;
        };
        if Money::spend_below(spend, max_budget.mul_decimal_percent(min_pct)) {
            return;
        }
        schedule(
            alerts,
            fields.build(BudgetAlertKind::MaxBudgetAlert, spend, Some(config)),
        );
        return;
    }
    let threshold = max_budget.mul(fraction);
    if Money::spend_at_least(spend, threshold) && Money::spend_below(spend, max_budget) {
        debug!(
            spend = %spend,
            max_budget = ?max_budget,
            threshold = ?threshold,
            "Reached Max Budget Alert Threshold for token"
        );
        schedule(
            alerts,
            fields.build(BudgetAlertKind::MaxBudgetAlert, spend, None),
        );
    }
}
