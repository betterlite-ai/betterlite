use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use thiserror::Error;
use tracing::debug;

use crate::resolved_key::auth::UserAPIKeyAuth;

use super::Admission;
use super::alerts::{BudgetAlertKind, BudgetAlertSink, KeyAlertFields, schedule};
use super::email::{merge_alert_email_configs, min_digit_threshold};
use super::key_budget::KeyBudgets;
use super::money::Money;

// LiteLLM's merge raises AttributeError on a truthy non-mapping; betterlite names the field and returns the auth error instead
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum AlertError {
    #[error("max_budget_alert_emails must be a mapping of thresholds to recipients")]
    NotAMapping,
}

// _virtual_key_max_budget_alert_check: a merged map alerts at its lowest digit threshold even past the maximum; otherwise the legacy fraction alerts only strictly below it
pub fn max_budget_alert_check(
    auth: &UserAPIKeyAuth,
    budgets: &KeyBudgets,
    admission: &Admission<'_>,
) -> Result<(), AlertError> {
    let Some(max_budget) = budgets.max_budget else {
        return Ok(());
    };
    let spend = auth.view.token.spend;
    if spend <= Decimal::ZERO {
        return Ok(());
    }
    let fields = KeyAlertFields {
        auth,
        max_budget: budgets.max_budget,
        soft_budget: budgets.soft_budget,
        user_email: admission.user_email,
        team_alias: auth.view.team_alias.as_deref(),
    };
    let Some(config) =
        merge_alert_email_configs(admission.global_alert_emails, per_key_config(auth)?)
    else {
        legacy_alert(
            &fields,
            spend,
            max_budget,
            admission.alert_fraction,
            admission.alerts,
        );
        return Ok(());
    };
    mapped_alert(&fields, spend, max_budget, config, admission.alerts);
    Ok(())
}

// `if not config` in LiteLLM: the JSON values Python reads as false
fn is_falsey(raw: &JsonData) -> bool {
    match raw {
        JsonData::Null => true,
        JsonData::Bool(flag) => !*flag,
        JsonData::Number(number) => number.as_f64().is_some_and(|value| value == 0.0_f64),
        JsonData::String(text) => text.is_empty(),
        JsonData::Array(items) => items.is_empty(),
        JsonData::Object(fields) => fields.is_empty(),
    }
}

fn per_key_config(
    auth: &UserAPIKeyAuth,
) -> Result<Option<&BTreeMap<String, JsonData>>, AlertError> {
    let config = auth.view.token.metadata.0.get("max_budget_alert_emails");
    let Some(value) = config.filter(|value| !is_falsey(value)) else {
        return Ok(None);
    };
    match value {
        JsonData::Object(fields) => Ok(Some(fields)),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => Err(AlertError::NotAMapping),
    }
}

fn mapped_alert(
    fields: &KeyAlertFields<'_>,
    spend: Decimal,
    max_budget: Money,
    config: BTreeMap<String, Vec<String>>,
    alerts: Option<&dyn BudgetAlertSink>,
) {
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
}

fn legacy_alert(
    fields: &KeyAlertFields<'_>,
    spend: Decimal,
    max_budget: Money,
    fraction: Money,
    alerts: Option<&dyn BudgetAlertSink>,
) {
    let threshold = max_budget.times(fraction);
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
