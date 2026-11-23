pub mod alerts;
pub mod email;
pub mod expiry;
pub mod key_budget;
pub mod max_alert;
pub mod money;
pub mod soft;
pub mod spend;
pub mod team_member;
pub mod temp;
mod tests;
pub mod windows;

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::budget_primitives::window_start::DurationParser;
use crate::resolved_key::auth::UserAPIKeyAuth;

use self::alerts::BudgetAlertSink;
use self::expiry::TempBudgetError;
use self::key_budget::{Denial, KeyBudgets, virtual_key_max_budget_check};
use self::money::Money;
use self::spend::{SpendReadError, SpendReader};
use self::temp::update_key_budget_with_temp_budget_increase;
use self::windows::{WindowError, virtual_key_multi_budget_check};

#[derive(Debug, Error)]
pub enum KeyBudgetError {
    #[error(transparent)]
    Denied(#[from] Denial),
    #[error(transparent)]
    Spend(#[from] SpendReadError),
    #[error(transparent)]
    Window(#[from] WindowError),
    #[error(transparent)]
    Temp(#[from] TempBudgetError),
}

// everything the checks need from the surrounding request: the reader and sink the proxy wired,
// the settings B002 resolved once, and the request clock
pub struct Admission<'request> {
    pub reader: &'request dyn SpendReader,
    pub alerts: Option<&'request dyn BudgetAlertSink>,
    pub durations: &'request dyn DurationParser,
    pub global_alert_emails: Option<&'request BTreeMap<String, JsonData>>,
    pub alert_fraction: Money,
    pub throttle_pct: Option<f64>,
    pub now: DateTime<Utc>,
    pub user_email: Option<&'request str>,
}

// the key maximum and soft budget the request enforces, with a temporary increase applied
pub fn effective_budgets(
    auth: &UserAPIKeyAuth,
    now: DateTime<Utc>,
) -> Result<KeyBudgets, TempBudgetError> {
    let update = update_key_budget_with_temp_budget_increase(auth, now)?;
    Ok(KeyBudgets::from_auth(auth).with_max_budget(update.max_budget))
}

// the builder's check order: the maximum alert, then the hard check, then the soft check, then the
// multi-window check; the hard denial stops the later checks
pub async fn admit(
    auth: &mut UserAPIKeyAuth,
    budgets: &KeyBudgets,
    admission: &Admission<'_>,
) -> Result<(), KeyBudgetError> {
    max_alert::virtual_key_max_budget_alert_check(
        auth,
        budgets,
        admission.user_email,
        admission.global_alert_emails,
        admission.alert_fraction,
        admission.alerts,
    );
    virtual_key_max_budget_check(
        auth,
        budgets,
        admission.user_email,
        admission.throttle_pct,
        admission.reader,
        admission.alerts,
    )
    .await?;
    soft::virtual_key_soft_budget_check(auth, budgets, admission.user_email, admission.alerts);
    virtual_key_multi_budget_check(auth, admission.reader, admission.durations, admission.now).await
}
