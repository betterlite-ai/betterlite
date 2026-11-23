use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use secrecy::ExposeSecret as _;

use crate::budget_primitives::throttle::{percentage, should_throttle};
use crate::current_spend::{SpendQuery, WindowDescriptors};
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::KeyBudgetError;
use super::alerts::{BudgetAlertKind, BudgetAlertSink, KeyAlertFields, schedule};
use super::money::Money;
use super::spend::SpendReader;

// litellm.budget_exceeded_status_code's default; B757 owns the configurable envelope
pub const BUDGET_EXCEEDED_STATUS: u16 = 422;

// _MASKED_KEY_NAME_RE, anchored because LiteLLM calls fullmatch
const MASKED_KEY_PREFIX: &str = "sk-...";
const MASKED_KEY_SUFFIX_CHARS: usize = 4;

// the maximum and soft budget a request enforces: read from the token, or handed in for a value
// the token's Decimal fields cannot hold, such as the nonfinite tests and a temporary increase
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyBudgets {
    pub max_budget: Option<Money>,
    pub soft_budget: Option<Money>,
}

impl KeyBudgets {
    #[must_use]
    pub fn from_auth(auth: &UserAPIKeyAuth) -> Self {
        Self {
            max_budget: auth.view.token.max_budget.map(Money::Finite),
            soft_budget: auth.view.soft_budget.map(Money::Finite),
        }
    }

    #[must_use]
    pub fn with_max_budget(self, max_budget: Option<Money>) -> Self {
        Self { max_budget, ..self }
    }
}

// the BudgetExceededError fields a caller can read: current_cost, max_budget, entity type and id
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Denial {
    pub current_cost: Decimal,
    pub max_budget: Decimal,
    pub entity_type: String,
    pub entity_id: String,
    pub message: String,
    pub status_code: u16,
}

pub fn token_text(auth: &UserAPIKeyAuth) -> String {
    auth.view.token.token.as_ref().map_or_else(
        || "None".to_owned(),
        |secret| secret.expose_secret().to_owned(),
    )
}

// _virtual_key_max_budget_check: no configured maximum makes no read and schedules no alert; a
// maximum the token can enforce is compared against the cross-pod spend, with the alert scheduled
// before the denial test
pub async fn virtual_key_max_budget_check(
    auth: &mut UserAPIKeyAuth,
    budgets: &KeyBudgets,
    user_email: Option<&str>,
    throttle_pct: Option<f64>,
    reader: &dyn SpendReader,
    alerts: Option<&dyn BudgetAlertSink>,
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
    let spend = reader.read_spend(&query).await?;
    let fields = KeyAlertFields {
        auth,
        max_budget: budgets.max_budget,
        soft_budget: budgets.soft_budget,
        user_email,
        team_alias: None,
    };
    schedule(
        alerts,
        fields.build(BudgetAlertKind::TokenBudget, spend, None),
    );
    if let Some(maximum) = max_budget.finite_value()
        && spend >= maximum
    {
        if apply_budget_exceeded_throttle(auth, throttle_pct) {
            return Ok(());
        }
        return Err(denial(auth, spend, maximum).into());
    }
    Ok(())
}

// _apply_budget_exceeded_throttle: the request-local percentage is recorded and the request is
// allowed; the token's configured TPM/RPM limits stay untouched
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
        entity_id: token_text(auth),
        message: format!(
            "Budget has been exceeded! Key={} Current cost: {spend}, Max budget: {max_budget}",
            key_descriptor(auth)
        ),
        status_code: BUDGET_EXCEEDED_STATUS,
    }
}

// the caller-facing key name: the alias, or "key", with a still-masked key_name added only when it
// fullmatches the abbreviation shape, so a raw key_name never leaves the proxy
fn key_descriptor(auth: &UserAPIKeyAuth) -> String {
    let token = &auth.view.token;
    let label = token
        .key_alias
        .as_deref()
        .filter(|alias| !alias.is_empty())
        .unwrap_or("key");
    match token.key_name.as_deref() {
        Some(name) if is_masked_key_name(name) => format!("{label} ({name})"),
        Some(_) | None => label.to_owned(),
    }
}

fn is_masked_key_name(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix(MASKED_KEY_PREFIX) else {
        return false;
    };
    if suffix.is_empty() {
        return true;
    }
    let characters: Vec<char> = suffix.chars().take(MASKED_KEY_SUFFIX_CHARS + 1).collect();
    characters.len() == MASKED_KEY_SUFFIX_CHARS
        && characters
            .iter()
            .all(|character| !character.is_whitespace() && !character.is_control())
}
