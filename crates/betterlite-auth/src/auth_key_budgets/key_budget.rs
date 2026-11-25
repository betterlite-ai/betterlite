use regex::Regex;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use secrecy::{ExposeSecret as _, SecretString};

use crate::budget_primitives::throttle::{percentage, should_throttle};
use crate::current_spend::{SpendQuery, WindowDescriptors};
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::alerts::{BudgetAlertKind, KeyAlertFields, schedule};
use super::money::Money;
use super::{Admission, KeyBudgetError};

// litellm.budget_exceeded_status_code's default; B757 owns the configurable envelope
pub const BUDGET_EXCEEDED_STATUS: u16 = 422;

// _MASKED_KEY_NAME_RE, anchored because LiteLLM calls fullmatch
const MASKED_KEY_NAME: &str = r"^sk-\.\.\.(?:[^\s\x00-\x1f\x7f-\x9f]{4})?$";

// the budgets a request enforces: the token's values, or the nonfinite ones its Decimal fields cannot hold
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyBudgets {
    pub max_budget: Option<Money>,
    pub soft_budget: Option<Money>,
}

// the BudgetExceededError fields a caller can read: current_cost, max_budget, entity type and id
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct Denial {
    pub current_cost: Decimal,
    pub max_budget: Decimal,
    pub entity_type: String,
    pub entity_id: Option<SecretString>,
    pub message: String,
    pub status_code: u16,
}

#[must_use]
pub fn token_text(auth: &UserAPIKeyAuth) -> String {
    auth.view.token.token.as_ref().map_or_else(
        || "None".to_owned(),
        |secret| secret.expose_secret().to_owned(),
    )
}

// _virtual_key_max_budget_check: no maximum skips the read and alert; the cross-pod spend is compared and the alert scheduled before the denial
pub async fn virtual_key_max_budget_check(
    auth: &mut UserAPIKeyAuth,
    budgets: &KeyBudgets,
    admission: &Admission<'_>,
) -> Result<(), KeyBudgetError> {
    let Some(max_budget) = budgets.max_budget else {
        return Ok(());
    };
    let counter_key = format!("spend:key:{}", token_text(auth));
    let query = SpendQuery {
        counter_key: &counter_key,
        fallback_spend: auth.view.token.spend,
        max_budget: max_budget.finite_value(),
        window: WindowDescriptors {
            entity_type: None,
            entity_id: None,
            duration: None,
            start: None,
        },
        fallback_authoritative: false,
    };
    let spend = admission.reader.read_spend(&query).await?;
    let fields = KeyAlertFields {
        auth,
        max_budget: budgets.max_budget,
        soft_budget: budgets.soft_budget,
        user_email: admission.user_email,
        team_alias: None,
    };
    schedule(
        admission.alerts,
        fields.build(BudgetAlertKind::TokenBudget, spend, None),
    );
    if let Some(maximum) = max_budget.finite_value()
        && spend >= maximum
    {
        if apply_budget_exceeded_throttle(auth, admission.throttle_pct) {
            return Ok(());
        }
        return Err(KeyBudgetError::Denied(denial(auth, spend, maximum)));
    }
    Ok(())
}

// _apply_budget_exceeded_throttle: records the request-local percentage and allows; the token's TPM/RPM limits stay untouched
#[must_use]
pub fn apply_budget_exceeded_throttle(auth: &mut UserAPIKeyAuth, configured: Option<f64>) -> bool {
    let Some(value) = percentage(configured) else {
        return false;
    };
    if !should_throttle(auth, configured) {
        return false;
    }
    let Some(recorded) = Decimal::from_f64(value) else {
        return false;
    };
    auth.extra.budget_throttle_pct = Some(recorded);
    true
}

fn denial(auth: &UserAPIKeyAuth, spend: Decimal, max_budget: Decimal) -> Denial {
    Denial {
        current_cost: spend,
        max_budget,
        entity_type: "key".to_owned(),
        entity_id: auth.view.token.token.clone(),
        message: format!(
            "Budget has been exceeded! Key={} Current cost: {spend}, Max budget: {max_budget}",
            key_descriptor(auth)
        ),
        status_code: BUDGET_EXCEEDED_STATUS,
    }
}

// the caller-facing key name: the alias or "key", with a still-masked key_name appended so a raw one never leaves
fn key_descriptor(auth: &UserAPIKeyAuth) -> String {
    let token = &auth.view.token;
    let label = token
        .key_alias
        .as_deref()
        .filter(|alias| !alias.is_empty())
        .unwrap_or("key");
    let masked_name = token
        .key_name
        .as_deref()
        .filter(|name| Regex::new(MASKED_KEY_NAME).is_ok_and(|pattern| pattern.is_match(name)));
    masked_name.map_or_else(|| label.to_owned(), |name| format!("{label} ({name})"))
}
