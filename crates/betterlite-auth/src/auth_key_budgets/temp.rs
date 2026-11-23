use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};

use crate::resolved_key::auth::UserAPIKeyAuth;

use super::expiry::{TempBudgetError, parse_temp_budget_expiry};
use super::money::Money;

// the copy _update_key_budget_with_temp_budget_increase returns: max_budget is the request's
// effective maximum, and applied records whether the metadata increase produced it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TempBudgetUpdate {
    pub max_budget: Option<Money>,
    pub applied: bool,
}

// _get_temp_budget_increase: both metadata fields are required, and only a future expiry applies
pub fn get_temp_budget_increase(
    auth: &UserAPIKeyAuth,
    now: DateTime<Utc>,
) -> Result<Option<Money>, TempBudgetError> {
    let metadata = &auth.view.token.metadata.0;
    let Some(increase) = metadata.get("temp_budget_increase") else {
        return Ok(None);
    };
    let Some(expiry) = metadata.get("temp_budget_expiry") else {
        return Ok(None);
    };
    let JsonData::String(expiry_text) = expiry else {
        return Err(TempBudgetError::ExpiryInvalid);
    };
    if parse_temp_budget_expiry(expiry_text)? > now {
        let increase = Money::from_json(increase).ok_or(TempBudgetError::IncreaseInvalid)?;
        return Ok(Some(increase));
    }
    Ok(None)
}

// _update_key_budget_with_temp_budget_increase: no configured maximum skips the parse entirely,
// and a falsey increase leaves the original maximum in place
pub fn update_key_budget_with_temp_budget_increase(
    auth: &UserAPIKeyAuth,
    now: DateTime<Utc>,
) -> Result<TempBudgetUpdate, TempBudgetError> {
    let Some(max_budget) = auth.view.token.max_budget else {
        return Ok(TempBudgetUpdate {
            max_budget: None,
            applied: false,
        });
    };
    let original = TempBudgetUpdate {
        max_budget: Some(Money::Finite(max_budget)),
        applied: false,
    };
    let Some(increase) = get_temp_budget_increase(auth, now)? else {
        return Ok(original);
    };
    if !increase.truthy() {
        return Ok(original);
    }
    Ok(TempBudgetUpdate {
        max_budget: Some(Money::Finite(max_budget).add(increase)),
        applied: true,
    })
}
